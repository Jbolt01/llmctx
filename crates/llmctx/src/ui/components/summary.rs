//! Selection summary component.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};

use crate::app::tokens::{BundleTokenSummary, ItemTokenEstimate};

/// Displays aggregated selection statistics including token usage.
#[derive(Debug, Default)]
pub struct Summary {
    latest: Option<BundleTokenSummary>,
}

impl Summary {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the stored summary with fresh data from the estimator.
    pub fn update(&mut self, summary: BundleTokenSummary) {
        self.latest = Some(summary);
    }

    /// Clear the rendered state when selections are emptied.
    pub fn clear(&mut self) {
        self.latest = None;
    }

    /// Render the summary inside the provided area.
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect) {
        let block = Block::default()
            .title("Selection Summary")
            .borders(Borders::ALL);
        frame.render_widget(block.clone(), area);

        let inner = block.inner(area);
        match &self.latest {
            Some(summary) => self.render_summary(frame, inner, summary),
            None => {
                let placeholder = Paragraph::new("No selections")
                    .wrap(Wrap { trim: true })
                    .style(Style::default().fg(Color::DarkGray));
                frame.render_widget(placeholder, inner);
            }
        }
    }

    fn render_summary(&self, frame: &mut Frame<'_>, area: Rect, summary: &BundleTokenSummary) {
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(4), Constraint::Min(1)])
            .split(area);

        let header = Paragraph::new(header_lines(summary)).wrap(Wrap { trim: true });
        frame.render_widget(header, layout[0]);

        let items = build_item_list(&summary.items);
        if items.is_empty() {
            let empty = Paragraph::new("No files selected").wrap(Wrap { trim: true });
            frame.render_widget(empty, layout[1]);
        } else {
            let list = List::new(items).block(Block::default());
            frame.render_widget(list, layout[1]);
        }
    }
}

fn header_lines(summary: &BundleTokenSummary) -> Vec<Line<'static>> {
    let usage_ratio = if summary.token_budget == 0 {
        0.0
    } else {
        summary.total_tokens as f64 / summary.token_budget as f64
    };
    let status_color = if summary.token_budget == 0 {
        Color::Green
    } else if summary.total_tokens as u32 >= summary.token_budget {
        Color::Red
    } else if usage_ratio >= 0.9 {
        Color::Yellow
    } else {
        Color::Green
    };

    let provider = format!("{} · {}", summary.model.provider(), summary.model.as_str());
    let total = format!("{} tokens", summary.total_tokens);
    let budget_text = if summary.token_budget == 0 {
        "unbounded".to_string()
    } else {
        format!("{} tokens", summary.token_budget)
    };
    let percent = if summary.token_budget == 0 {
        "0%".to_string()
    } else {
        format!("{:.0}%", (usage_ratio * 100.0).clamp(0.0, 999.0))
    };

    vec![
        Line::from(vec![
            Span::styled("Model", Style::default().fg(Color::Gray)),
            Span::raw(": "),
            Span::styled(provider, Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::styled("Usage", Style::default().fg(Color::Gray)),
            Span::raw(": "),
            Span::styled(total, Style::default().fg(status_color)),
            Span::raw(" / "),
            Span::raw(budget_text),
            Span::raw(" ("),
            Span::styled(percent, Style::default().fg(status_color)),
            Span::raw(")"),
        ]),
        Line::from(vec![
            Span::styled("Characters", Style::default().fg(Color::Gray)),
            Span::raw(": "),
            Span::raw(format!("{}", summary.total_characters)),
        ]),
    ]
}

fn build_item_list(items: &[ItemTokenEstimate]) -> Vec<ListItem<'static>> {
    items
        .iter()
        .map(|item| {
            let mut label = item.item.path.display().to_string();
            if let Some((start, end)) = item.item.range {
                label.push_str(&format!(" [{start}-{end}]"));
            }
            label.push_str(&format!(" – {} tokens", item.tokens));
            let mut spans = vec![Span::raw(label)];
            if let Some(note) = &item.item.note {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!("({})", note.replace('\n', " ")),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::app::tokens::TokenModel;
    use crate::domain::model::SelectionItem;

    #[test]
    fn renders_empty_state_without_summary() {
        let backend = TestBackend::new(40, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let summary = Summary::new();
        terminal
            .draw(|frame| {
                let area = frame.size();
                summary.render(frame, area);
            })
            .unwrap();
    }

    #[test]
    fn renders_summary_with_items() {
        let backend = TestBackend::new(60, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut summary = Summary::new();

        let data = BundleTokenSummary {
            model: TokenModel::CharacterFallback,
            token_budget: 1_000,
            total_tokens: 120,
            total_characters: 480,
            items: vec![ItemTokenEstimate {
                item: SelectionItem {
                    path: "path/to/file.rs".into(),
                    range: Some((1, 5)),
                    note: Some("example".into()),
                },
                tokens: 120,
                characters: 480,
            }],
        };
        summary.update(data);

        terminal
            .draw(|frame| {
                let area = frame.size();
                summary.render(frame, area);
            })
            .unwrap();
    }
}
