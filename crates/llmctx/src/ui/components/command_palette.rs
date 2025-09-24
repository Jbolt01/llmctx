//! Command palette component for quick actions.

use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

/// Interactive state backing the command palette overlay.
#[derive(Debug, Default, Clone)]
pub struct CommandPaletteState {
    visible: bool,
    input: String,
    message: Option<PaletteMessage>,
}

impl CommandPaletteState {
    /// Reveal the palette with an empty input buffer.
    pub fn open(&mut self) {
        self.visible = true;
        self.input.clear();
    }

    /// Reveal the palette with an initial command prefilled.
    pub fn open_with<S: Into<String>>(&mut self, content: S) {
        self.visible = true;
        self.input = content.into();
    }

    /// Hide the palette.
    pub fn close(&mut self) {
        self.visible = false;
    }

    /// Whether the palette is currently displayed.
    pub fn is_open(&self) -> bool {
        self.visible
    }

    /// Access the current input buffer.
    pub fn input(&self) -> &str {
        &self.input
    }

    /// Replace the current input contents.
    pub fn set_input<S: Into<String>>(&mut self, content: S) {
        self.input = content.into();
    }

    /// Consume the current input, leaving the buffer empty.
    pub fn take_input(&mut self) -> String {
        std::mem::take(&mut self.input)
    }

    /// Append a character to the buffer.
    pub fn push_char(&mut self, ch: char) {
        self.input.push(ch);
    }

    /// Remove the most recently appended character if present.
    pub fn pop_char(&mut self) {
        self.input.pop();
    }

    /// Record a status message to display beneath the input field.
    pub fn set_message<S: Into<String>>(&mut self, level: PaletteMessageLevel, message: S) {
        self.message = Some(PaletteMessage::new(level, message.into()));
    }

    /// Clear any displayed message.
    pub fn clear_message(&mut self) {
        self.message = None;
    }

    /// Retain only messages that have not expired.
    pub fn purge_expired_messages(&mut self) {
        if let Some(message) = &self.message
            && message.is_expired()
        {
            self.message = None;
        }
    }
}

/// Visual component that renders the command palette overlay.
#[derive(Debug, Default)]
pub struct CommandPalette;

impl CommandPalette {
    /// Draw the palette if it is visible.
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, state: &CommandPaletteState) {
        if !state.is_open() {
            return;
        }

        let width = area.width.saturating_sub(10).min(80);
        let popup = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + area.height.saturating_sub(6),
            width,
            height: 5,
        };

        frame.render_widget(Clear, popup);

        let block = Block::default()
            .title("Command Palette")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));
        frame.render_widget(block.clone(), popup);

        let inner = block.inner(popup);
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(0)])
            .split(inner);

        let prompt = Paragraph::new(format!(":{}", state.input()))
            .style(Style::default().fg(Color::White))
            .block(Block::default());
        frame.render_widget(prompt, layout[0]);

        if let Some(message) = &state.message {
            let style = match message.level {
                PaletteMessageLevel::Info => Style::default().fg(Color::Gray),
                PaletteMessageLevel::Success => Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
                PaletteMessageLevel::Error => {
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
                }
            };
            let paragraph = Paragraph::new(Line::from(message.text.clone()))
                .wrap(Wrap { trim: true })
                .style(style);
            frame.render_widget(paragraph, layout[1]);
        }
    }
}

/// Command palette message severity levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteMessageLevel {
    Info,
    Success,
    Error,
}

#[derive(Debug, Clone)]
struct PaletteMessage {
    level: PaletteMessageLevel,
    text: String,
    expires_at: Instant,
}

impl PaletteMessage {
    fn new(level: PaletteMessageLevel, text: String) -> Self {
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
