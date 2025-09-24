//! Preview component rendering highlighted file segments.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::preview::PreviewSegment;
use crate::infra::highlight::HighlightSpan;

/// Ratatui component responsible for displaying file previews with line numbers.
#[derive(Debug, Default)]
pub struct Preview;

impl Preview {
    pub fn render(&self, segment: &PreviewSegment, area: Rect, buf: &mut Buffer) {
        let title = format!(
            "{} ({}-{})",
            segment.path.display(),
            segment.start_line,
            segment.end_line
        );

        let block = Block::default().title(title).borders(Borders::ALL);
        let inner = block.inner(area);
        block.render(area, buf);

        let mut lines = Vec::with_capacity(segment.highlighted.lines.len());
        for (idx, line) in segment.highlighted.lines.iter().enumerate() {
            let line_number = segment.start_line + idx;
            let prefix = format!("{:>4} │ ", line_number);
            let mut spans = vec![Span::styled(prefix, Style::default().fg(Color::DarkGray))];
            spans.extend(line.spans.iter().map(highlight_span_to_span));
            lines.push(Line::from(spans));
        }

        if let Some(notice) = &segment.notice {
            lines.insert(
                0,
                Line::styled(
                    notice,
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
            );
        }

        if segment.truncated {
            lines.push(Line::styled(
                "… truncated; press → to load more",
                Style::default().fg(Color::Yellow),
            ));
        }

        if let Some(token) = &segment.continuation {
            lines.push(Line::styled(
                format!("press enter to load from line {}", token.start_line + 1),
                Style::default().fg(Color::Cyan),
            ));
        }

        if lines.is_empty() {
            lines.push(Line::styled(
                "(empty file)",
                Style::default().fg(Color::DarkGray),
            ));
        }

        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        ratatui::widgets::Widget::render(paragraph, inner, buf);
    }
}

fn highlight_span_to_span(span: &HighlightSpan) -> Span<'_> {
    let mut style = Style::default();

    if let Some(color) = span.style.foreground {
        style = style.fg(Color::Rgb(color.r, color.g, color.b));
    }
    if let Some(color) = span.style.background {
        style = style.bg(Color::Rgb(color.r, color.g, color.b));
    }

    if span.style.attributes.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    if span.style.attributes.italic {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if span.style.attributes.underline {
        style = style.add_modifier(Modifier::UNDERLINED);
    }

    Span::styled(span.content.clone(), style)
}
