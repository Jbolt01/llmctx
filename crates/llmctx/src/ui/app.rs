//! Application loop for the TUI.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use time::OffsetDateTime;
use time::macros::format_description;

use crate::app::export::{ExportOptions, Exporter};
use crate::app::preview::{PreviewSegment, PreviewService};
use crate::app::scan::{ScanResult, Scanner, ScannerConfig};
use crate::app::selection::SelectionManager;
use crate::app::session::{SelectionRecord, SessionSnapshot, SessionStore};
use crate::app::tokens::{BundleTokenSummary, TokenEstimator};
use crate::infra::config::Config;
use crate::ui::components::command_palette::{CommandPalette, CommandPaletteState};
use crate::ui::components::file_tree::{FileTree, FileTreeState};
use crate::ui::components::preview::Preview;
use crate::ui::components::summary::Summary;

const TICK_RATE: Duration = Duration::from_millis(120);

/// Primary entry point for running the interactive TUI.
pub struct UiApp {
    config: Config,
    scanner: Scanner,
    scan: Option<ScanResult>,
    tree: FileTreeState,
    file_tree: FileTree,
    preview_service: PreviewService,
    preview: PreviewState,
    selection: SelectionManager,
    token_estimator: TokenEstimator,
    summary_component: Summary,
    last_summary: Option<BundleTokenSummary>,
    session_store: SessionStore,
    palette_state: CommandPaletteState,
    palette_component: CommandPalette,
    exporter: Exporter,
    selected_paths: HashSet<String>,
    path_lookup: HashMap<PathBuf, String>,
    status: Option<StatusMessage>,
    focus: FocusTarget,
    should_quit: bool,
}

impl Default for UiApp {
    fn default() -> Self {
        Self {
            config: Config::default(),
            scanner: Scanner::new(),
            scan: None,
            tree: FileTreeState::default(),
            file_tree: FileTree,
            preview_service: PreviewService::new(),
            preview: PreviewState::default(),
            selection: SelectionManager::new(),
            token_estimator: TokenEstimator::default(),
            summary_component: Summary::new(),
            last_summary: None,
            session_store: SessionStore::new(PathBuf::from(".")),
            palette_state: CommandPaletteState::default(),
            palette_component: CommandPalette,
            exporter: Exporter::new().expect("exporter available"),
            selected_paths: HashSet::new(),
            path_lookup: HashMap::new(),
            status: None,
            focus: FocusTarget::FileTree,
            should_quit: false,
        }
    }
}

impl UiApp {
    /// Launch the terminal UI and enter the event loop.
    pub fn run(&mut self) -> Result<()> {
        self.bootstrap()?;

        enable_raw_mode().context("failed to enable raw mode")?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;

        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend).context("failed to initialize terminal")?;
        terminal.hide_cursor().ok();

        let event_loop_result = self.event_loop(&mut terminal);

        disable_raw_mode().ok();
        let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
        let _ = terminal.show_cursor();

        event_loop_result
    }

    fn bootstrap(&mut self) -> Result<()> {
        self.config = Config::load()?;
        let root = std::env::current_dir().context("unable to determine working directory")?;
        self.session_store = SessionStore::new(&root);

        let mut scanner_cfg = ScannerConfig::from_root(root.clone(), self.config.clone());
        scanner_cfg = scanner_cfg.with_max_file_size(2 * 1024 * 1024);
        let scan = self
            .scanner
            .scan(&scanner_cfg)
            .context("failed to scan workspace")?;
        self.path_lookup = scan
            .files
            .iter()
            .map(|meta| (meta.path.clone(), meta.display_path.clone()))
            .collect();
        self.tree = FileTreeState::from_scan(&scan);
        self.scan = Some(scan);

        self.token_estimator = TokenEstimator::from_config(&self.config);
        self.preview_service = PreviewService::new();
        self.exporter = Exporter::new()?;

        if let Some(snapshot) = self.session_store.load()? {
            self.restore_session(snapshot)?;
        }

        self.refresh_selection_state()?;
        Ok(())
    }

    fn event_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
        loop {
            terminal.draw(|frame| self.render(frame))?;
            self.tick();

            if self.should_quit {
                break;
            }

            if event::poll(TICK_RATE)? {
                let ev = event::read()?;
                self.handle_event(ev)?;
            }
        }
        Ok(())
    }

    fn render(&mut self, frame: &mut Frame<'_>) {
        let size = frame.size();
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(1)])
            .split(size);

        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(32),
                Constraint::Min(50),
                Constraint::Length(36),
            ])
            .split(layout[0]);

        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(10), Constraint::Length(5)])
            .split(main_chunks[2]);

        let focus_tree = matches!(self.focus, FocusTarget::FileTree);
        let focus_preview = matches!(self.focus, FocusTarget::Preview);

        let selected_paths = &self.selected_paths;
        self.file_tree.render(
            frame,
            main_chunks[0],
            &self.tree,
            focus_tree,
            selected_paths,
        );

        if let Some(segment) = self.preview.segment() {
            self.preview_component().render(
                segment,
                self.preview.highlight_ranges(),
                focus_preview,
                main_chunks[1],
                frame.buffer_mut(),
            );
        } else {
            let block = Block::default()
                .title("Preview")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(if focus_preview {
                    Color::Cyan
                } else {
                    Color::DarkGray
                }));
            let inner = block.inner(main_chunks[1]);
            frame.render_widget(block, main_chunks[1]);
            let placeholder = Paragraph::new("Select a file to preview")
                .style(
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                )
                .wrap(Wrap { trim: true });
            frame.render_widget(placeholder, inner);
        }

        self.summary_component.render(frame, right_chunks[0]);

        let hints = Paragraph::new(Line::from(vec![
            Span::styled("j/k", Style::default().fg(Color::Cyan)),
            Span::raw(" move "),
            Span::styled("↵", Style::default().fg(Color::Cyan)),
            Span::raw(" preview · "),
            Span::styled("space", Style::default().fg(Color::Cyan)),
            Span::raw(" toggle select · "),
            Span::styled("/", Style::default().fg(Color::Cyan)),
            Span::raw(" filter · "),
            Span::styled(":", Style::default().fg(Color::Cyan)),
            Span::raw(" palette · "),
            Span::styled("ctrl+s", Style::default().fg(Color::Cyan)),
            Span::raw(" save · "),
            Span::styled("ctrl+e", Style::default().fg(Color::Cyan)),
            Span::raw(" export"),
        ]))
        .wrap(Wrap { trim: true })
        .style(Style::default().fg(Color::Gray));
        frame.render_widget(hints, right_chunks[1]);

        self.render_status(frame, layout[1]);
        self.palette_component
            .render(frame, size, &self.palette_state);
    }

    fn preview_component(&self) -> &Preview {
        static PREVIEW: Preview = Preview;
        &PREVIEW
    }

    fn render_status(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let message = self.status.as_ref().map(|status| {
            let style = match status.level {
                StatusLevel::Info => Style::default().fg(Color::Gray),
                StatusLevel::Success => Style::default().fg(Color::Green),
                StatusLevel::Error => Style::default().fg(Color::Red),
            };
            Line::styled(status.text.clone(), style)
        });

        let block = Block::default().borders(Borders::TOP);
        frame.render_widget(block.clone(), area);
        let inner = block.inner(area);

        let line = message.unwrap_or_else(|| {
            Line::styled(
                "Ready · press : for commands",
                Style::default().fg(Color::DarkGray),
            )
        });
        frame.render_widget(Paragraph::new(line), inner);
    }

    fn tick(&mut self) {
        if let Some(status) = &self.status
            && status.is_expired()
        {
            self.status = None;
        }
        self.palette_state.purge_expired_messages();
    }

    fn handle_event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::Key(key) => self.handle_key_event(key)?,
            Event::Resize(..) => {}
            Event::Mouse(_) => {}
            Event::FocusGained | Event::FocusLost | Event::Paste(_) => {}
        }
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Result<()> {
        if self.palette_state.is_open() {
            return self.handle_palette_key(key);
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('c') | KeyCode::Char('q') => {
                    self.should_quit = true;
                    return Ok(());
                }
                KeyCode::Char('s') => {
                    self.save_session()?;
                    return Ok(());
                }
                KeyCode::Char('e') => {
                    self.perform_export(None, true)?;
                    return Ok(());
                }
                _ => {}
            }
        }

        match self.focus {
            FocusTarget::FileTree => self.handle_tree_key(key),
            FocusTarget::Preview => self.handle_preview_key(key),
            FocusTarget::CommandPalette => Ok(()),
        }
    }

    fn handle_tree_key(&mut self, key: KeyEvent) -> Result<()> {
        if self.tree.is_filter_active() {
            return self.handle_filter_input(key);
        }

        match key.code {
            KeyCode::Esc => {
                self.should_quit = true;
            }
            KeyCode::Char('/') => {
                self.tree.begin_filter();
            }
            KeyCode::Char(':') => {
                self.palette_state.open();
                self.focus = FocusTarget::CommandPalette;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.tree.select_next();
                self.preview_current(false)?;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.tree.select_previous();
                self.preview_current(false)?;
            }
            KeyCode::Char('h') | KeyCode::Left => {
                self.tree.collapse_or_parent();
            }
            KeyCode::Char('l') | KeyCode::Right => {
                if self.preview_current(true)? {
                    self.focus = FocusTarget::Preview;
                } else {
                    self.tree.expand_or_open();
                }
            }
            KeyCode::Enter => {
                if self.preview_current(true)? {
                    self.focus = FocusTarget::Preview;
                }
            }
            KeyCode::Char(' ') => {
                self.toggle_current_selection()?;
            }
            KeyCode::Tab => {
                self.focus = FocusTarget::Preview;
            }
            KeyCode::Char('q') => {
                self.should_quit = true;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_preview_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.preview.clear_anchor();
                self.focus = FocusTarget::FileTree;
            }
            KeyCode::Char(':') => {
                self.palette_state.open();
                self.focus = FocusTarget::CommandPalette;
            }
            KeyCode::Char(' ') => {
                self.toggle_current_selection()?;
            }
            KeyCode::Tab | KeyCode::Left => {
                self.preview.clear_anchor();
                self.focus = FocusTarget::FileTree;
            }
            KeyCode::Right => {
                if self
                    .preview
                    .load_more(&self.preview_service, &self.config)?
                {
                    self.refresh_preview_highlights();
                }
            }
            KeyCode::Char('q') => {
                self.should_quit = true;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(change) = self
                    .preview
                    .move_cursor(-1, key.modifiers.contains(KeyModifiers::SHIFT))?
                {
                    self.apply_range_change(change)?;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    if let Some(change) = self.preview.move_cursor(1, true)? {
                        self.apply_range_change(change)?;
                    }
                } else {
                    if self.preview.at_bottom()
                        && self
                            .preview
                            .load_more(&self.preview_service, &self.config)?
                    {
                        self.refresh_preview_highlights();
                    }
                    if let Some(change) = self.preview.move_cursor(1, false)? {
                        self.apply_range_change(change)?;
                    }
                }
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.save_session()?;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_palette_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.palette_state.close();
                self.focus = FocusTarget::FileTree;
            }
            KeyCode::Enter => {
                let command = self.palette_state.take_input();
                self.palette_state.close();
                self.focus = FocusTarget::FileTree;
                if let Err(err) = self.execute_command(command.trim()) {
                    self.set_status(StatusLevel::Error, err.to_string());
                }
            }
            KeyCode::Backspace => {
                self.palette_state.pop_char();
            }
            KeyCode::Char(ch) => {
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                {
                    self.palette_state.push_char(ch);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_filter_input(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.tree.end_filter();
            }
            KeyCode::Enter => {
                self.tree.end_filter();
            }
            KeyCode::Backspace => {
                self.tree.pop_filter_char();
            }
            KeyCode::Char(ch) => {
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                {
                    self.tree.push_filter_char(ch);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn preview_current(&mut self, force: bool) -> Result<bool> {
        let metadata = match self.tree.selected_metadata() {
            Some(meta) => meta,
            None => return Ok(false),
        };
        if metadata.is_dir {
            return Ok(false);
        }

        if metadata.skipped.is_some() {
            self.set_status(
                StatusLevel::Info,
                format!("{} skipped during scan", metadata.display_path),
            );
            return Ok(false);
        }

        let already_previewing = self
            .preview
            .path()
            .map(|path| path == metadata.path)
            .unwrap_or(false);
        if already_previewing && !force {
            return Ok(true);
        }

        let segment = self
            .preview_service
            .preview(&metadata.path, None, &self.config)
            .with_context(|| format!("failed to preview {}", metadata.display_path))?;

        self.preview.set_segment(segment);
        self.refresh_preview_highlights();
        if force {
            self.focus = FocusTarget::Preview;
        }
        Ok(true)
    }

    fn refresh_preview_highlights(&mut self) {
        if let Some(path) = self.preview.path().map(PathBuf::from) {
            let mut ranges = Vec::new();
            for item in self.selection.items() {
                if item.path == path {
                    if let Some(range) = item.range {
                        ranges.push(range);
                    } else {
                        ranges.push((1, usize::MAX));
                    }
                }
            }
            self.preview.set_highlights(ranges);
        }
    }

    fn apply_range_change(&mut self, change: RangeChange) -> Result<()> {
        let RangeChange {
            path,
            removed,
            added,
        } = change;
        if let Some(range) = removed {
            self.selection.remove_selection(&path, Some(range));
        }
        if let Some(range) = added {
            self.selection
                .add_selection(path.clone(), Some(range), None);
        }
        self.refresh_selection_state()
    }

    fn toggle_current_selection(&mut self) -> Result<()> {
        let metadata = match self.tree.selected_metadata() {
            Some(meta) => meta,
            None => return Ok(()),
        };
        if metadata.is_dir {
            return Ok(());
        }

        let existed = self.selection.remove_selection(&metadata.path, None);
        if !existed {
            self.selection
                .add_selection(metadata.path.clone(), None, None);
            self.set_status(
                StatusLevel::Success,
                format!("Added {}", metadata.display_path),
            );
        } else {
            self.set_status(
                StatusLevel::Info,
                format!("Removed {}", metadata.display_path),
            );
        }
        self.refresh_selection_state()?;
        Ok(())
    }

    fn execute_command(&mut self, command: &str) -> Result<()> {
        if command.is_empty() {
            return Ok(());
        }

        let mut parts = command.split_whitespace();
        let verb = parts.next().unwrap_or("");
        let rest = command[verb.len()..].trim();

        match verb {
            "filter" => {
                self.tree.set_filter(rest);
                self.set_status(StatusLevel::Success, "Filter applied");
            }
            "clear" => {
                if rest == "filter" || rest.is_empty() {
                    self.tree.clear_filter();
                    self.set_status(StatusLevel::Info, "Filter cleared");
                }
            }
            "select" => {
                let range = parse_range(rest).ok_or_else(|| anyhow!("invalid range"))?;
                let segment = self
                    .preview
                    .segment()
                    .ok_or_else(|| anyhow!("open a preview first"))?;
                self.selection
                    .add_selection(segment.path.clone(), Some(range), None);
                self.set_status(
                    StatusLevel::Success,
                    format!(
                        "Selected {}:{}-{}",
                        segment.path.display(),
                        range.0,
                        range.1
                    ),
                );
                self.refresh_selection_state()?;
            }
            "export" => {
                if rest.is_empty() {
                    self.perform_export(None, true)?;
                } else {
                    self.perform_export(Some(PathBuf::from(rest)), true)?;
                }
            }
            "save" => {
                self.save_session()?;
            }
            "model" => {
                if rest.is_empty() {
                    return Err(anyhow!("model command requires an identifier"));
                }
                self.selection.set_model(rest.to_string());
                self.refresh_selection_state()?;
                self.set_status(StatusLevel::Success, format!("Model set to {rest}"));
            }
            "help" => {
                self.set_status(
                    StatusLevel::Info,
                    "Commands: filter, select <start-end>, export [path], save, model <id>",
                );
            }
            other => {
                return Err(anyhow!("unknown command '{other}'"));
            }
        }
        Ok(())
    }

    fn perform_export(&mut self, target: Option<PathBuf>, copy: bool) -> Result<()> {
        if self.selection.is_empty() {
            self.set_status(StatusLevel::Error, "No selections to export");
            return Ok(());
        }

        let mut options = ExportOptions::from_config(&self.config);
        options.copy_to_clipboard = copy;

        let path = if let Some(path) = target {
            path
        } else {
            let snapshot = self
                .session_store
                .path()
                .parent()
                .map(|dir| dir.join("exports"))
                .unwrap_or_else(|| PathBuf::from(".llmctx/exports"));
            fs::create_dir_all(&snapshot).context("failed to create export directory")?;
            let timestamp = OffsetDateTime::now_utc().format(format_description!(
                "[year][month][day]-[hour][minute][second]"
            ))?;
            snapshot.join(format!(
                "context-{timestamp}.{}",
                options.format.extension()
            ))
        };
        options.output_path = Some(path.clone());

        let summary = self.selection.summarize_tokens(&self.token_estimator)?;
        if let Some(ref data) = summary {
            self.summary_component.update(data.clone());
            self.last_summary = Some(data.clone());
        }

        let bundle = self.selection.to_bundle();
        self.exporter.export(&bundle, summary.as_ref(), &options)?;

        self.set_status(
            StatusLevel::Success,
            format!("Exported selection to {}", path.display()),
        );
        Ok(())
    }

    fn save_session(&mut self) -> Result<()> {
        let root = self
            .scan
            .as_ref()
            .map(|scan| scan.root.clone())
            .unwrap_or_else(|| PathBuf::from("."));
        let selections: Vec<SelectionRecord> = self
            .selection
            .items()
            .iter()
            .map(|item| {
                let mut record = SelectionRecord::from(item);
                if let Ok(relative) = item.path.strip_prefix(&root) {
                    record.path = relative.display().to_string();
                }
                record
            })
            .collect();
        let focused = self
            .tree
            .selected_metadata()
            .map(|meta| meta.display_path.clone());
        let filter = if self.tree.filter().is_empty() {
            None
        } else {
            Some(self.tree.filter().to_string())
        };
        let snapshot = SessionSnapshot {
            selections,
            focused_path: focused,
            filter,
            model: self.selection.model().map(ToString::to_string),
        };
        self.session_store.save(&snapshot)?;
        self.set_status(StatusLevel::Success, "Session saved");
        Ok(())
    }

    fn restore_session(&mut self, snapshot: SessionSnapshot) -> Result<()> {
        if let Some(model) = snapshot.model {
            self.selection.set_model(model);
        }
        let root = self
            .scan
            .as_ref()
            .map(|scan| scan.root.clone())
            .unwrap_or_else(|| PathBuf::from("."));
        for record in snapshot.selections {
            let mut item = record.into_selection_item();
            if item.path.is_relative() {
                item.path = root.join(item.path);
            }
            self.selection
                .add_selection(item.path.clone(), item.range, item.note.clone());
        }
        if let Some(filter) = snapshot.filter {
            self.tree.set_filter(filter);
        }
        if let Some(path) = snapshot.focused_path {
            self.tree.focus_path(&path);
            self.preview_current(false)?;
        }
        Ok(())
    }

    fn refresh_selection_state(&mut self) -> Result<()> {
        self.rebuild_selected_paths();
        self.refresh_preview_highlights();

        match self.selection.summarize_tokens(&self.token_estimator)? {
            Some(summary) => {
                self.summary_component.update(summary.clone());
                self.last_summary = Some(summary);
            }
            None => {
                self.summary_component.clear();
                self.last_summary = None;
            }
        }
        Ok(())
    }

    fn rebuild_selected_paths(&mut self) {
        self.selected_paths.clear();
        let root = self
            .scan
            .as_ref()
            .map(|scan| scan.root.clone())
            .unwrap_or_else(|| PathBuf::from("."));
        for item in self.selection.items() {
            let display = self
                .path_lookup
                .get(&item.path)
                .cloned()
                .unwrap_or_else(|| path_relative_to(&item.path, &root));
            self.selected_paths.insert(display);
        }
    }

    fn set_status<S: Into<String>>(&mut self, level: StatusLevel, message: S) {
        self.status = Some(StatusMessage::new(level, message.into()));
    }
}

fn path_relative_to(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn parse_range(input: &str) -> Option<(usize, usize)> {
    let (start, end) = input.split_once('-')?;
    let start = start.trim().parse().ok()?;
    let end = end.trim().parse().ok()?;
    Some((start, end))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusTarget {
    FileTree,
    Preview,
    CommandPalette,
}

#[derive(Debug)]
struct StatusMessage {
    level: StatusLevel,
    text: String,
    expires_at: Instant,
}

impl StatusMessage {
    fn new(level: StatusLevel, text: String) -> Self {
        Self {
            level,
            text,
            expires_at: Instant::now() + Duration::from_secs(4),
        }
    }

    fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }
}

#[derive(Debug, Clone, Copy)]
enum StatusLevel {
    Info,
    Success,
    Error,
}

#[derive(Debug, Default)]
struct PreviewState {
    segment: Option<PreviewSegment>,
    cursor: Option<usize>,
    anchor: Option<usize>,
    highlights: Vec<(usize, usize)>,
    active_range: Option<(usize, usize)>,
    active_path: Option<PathBuf>,
}

impl PreviewState {
    fn segment(&self) -> Option<&PreviewSegment> {
        self.segment.as_ref()
    }

    fn set_segment(&mut self, segment: PreviewSegment) {
        self.cursor = Some(segment.start_line);
        self.anchor = None;
        self.segment = Some(segment);
        self.active_range = None;
        self.active_path = None;
    }

    fn set_highlights(&mut self, highlights: Vec<(usize, usize)>) {
        self.highlights = highlights;
    }

    fn highlight_ranges(&self) -> &[(usize, usize)] {
        &self.highlights
    }

    fn path(&self) -> Option<&Path> {
        self.segment.as_ref().map(|segment| segment.path.as_path())
    }

    fn load_more(&mut self, service: &PreviewService, config: &Config) -> Result<bool> {
        let segment = match &self.segment {
            Some(segment) => segment.clone(),
            None => return Ok(false),
        };
        let token = match segment.continuation.clone() {
            Some(token) => token,
            None => return Ok(false),
        };
        let mut step = config.defaults.preview_max_lines();
        if step == 0 {
            step = 200;
        }
        let range = token.start_line..token.start_line + step;
        let next = service.preview(&segment.path, Some(range), config)?;
        self.cursor = Some(next.start_line);
        self.anchor = None;
        self.segment = Some(next);
        self.active_range = None;
        self.active_path = None;
        Ok(true)
    }

    fn move_cursor(&mut self, delta: isize, extend: bool) -> Result<Option<RangeChange>> {
        let segment = match &self.segment {
            Some(segment) => segment.clone(),
            None => return Ok(None),
        };
        let mut cursor = self.cursor.unwrap_or(segment.start_line);
        let prev_cursor = cursor;
        let min = segment.start_line;
        let max = segment.end_line.max(segment.start_line);
        cursor = cursor.saturating_add_signed(delta);
        cursor = cursor.clamp(min, max);

        if extend {
            let anchor = self.anchor.unwrap_or(prev_cursor);
            self.anchor = Some(anchor);
            let range = if cursor >= anchor {
                (anchor, cursor)
            } else {
                (cursor, anchor)
            };
            let path = segment.path.clone();
            if self.active_path.as_ref() != Some(&path) {
                self.active_path = Some(path.clone());
                self.active_range = None;
            }
            let change = RangeChange {
                path,
                removed: self.active_range,
                added: Some(range),
            };
            self.active_range = Some(range);
            self.cursor = Some(cursor);
            return Ok(Some(change));
        } else {
            self.anchor = None;
            self.active_range = None;
        }

        self.cursor = Some(cursor);
        Ok(None)
    }

    fn clear_anchor(&mut self) {
        self.anchor = None;
        self.active_range = None;
    }

    fn at_bottom(&self) -> bool {
        match (&self.segment, self.cursor) {
            (Some(segment), Some(cursor)) => cursor >= segment.end_line,
            _ => false,
        }
    }
}

#[derive(Debug)]
struct RangeChange {
    path: PathBuf,
    removed: Option<(usize, usize)>,
    added: Option<(usize, usize)>,
}

impl TokenEstimator {
    fn default() -> Self {
        TokenEstimator::from_config(&Config::default())
    }
}
