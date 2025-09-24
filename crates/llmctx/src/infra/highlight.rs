//! Syntax highlighting utilities built on top of syntect.

use std::borrow::Cow;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use once_cell::sync::Lazy;
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Style as SyntectStyle, Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};

const DEFAULT_THEME: &str = "base16-ocean.dark";

static DEFAULT_ASSETS: Lazy<(Arc<SyntaxSet>, Arc<ThemeSet>)> = Lazy::new(|| {
    let syntax_set = SyntaxSet::load_defaults_newlines();
    let mut theme_set = ThemeSet::load_defaults();

    for (name, source) in EMBEDDED_THEMES {
        if theme_set.themes.contains_key(*name) {
            continue;
        }
        let mut cursor = Cursor::new(source);
        match ThemeSet::load_from_reader(&mut cursor) {
            Ok(theme) => {
                theme_set.themes.insert((*name).to_string(), theme);
            }
            Err(err) => {
                tracing::warn!(theme = %name, error = %err, "failed to load embedded theme");
            }
        }
    }

    (Arc::new(syntax_set), Arc::new(theme_set))
});

/// Embedded themes bundled with the binary.
static EMBEDDED_THEMES: &[(&str, &str)] = &[(
    "dracula",
    include_str!("../../assets/themes/dracula.tmTheme"),
)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HighlightAttributes {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HighlightStyle {
    pub foreground: Option<RgbColor>,
    pub background: Option<RgbColor>,
    pub attributes: HighlightAttributes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightSpan {
    pub content: String,
    pub style: HighlightStyle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightLine {
    pub spans: Vec<HighlightSpan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightMode {
    Highlighted,
    Plain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightResult {
    pub lines: Vec<HighlightLine>,
    pub language: Option<String>,
    pub theme: String,
    pub mode: HighlightMode,
}

impl HighlightResult {
    pub fn plain(lines: Vec<String>, theme: String) -> Self {
        HighlightResult {
            lines: lines
                .into_iter()
                .map(|line| HighlightLine {
                    spans: vec![HighlightSpan {
                        content: line,
                        style: HighlightStyle::default(),
                    }],
                })
                .collect(),
            language: None,
            theme,
            mode: HighlightMode::Plain,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Highlighter {
    syntax_set: Arc<SyntaxSet>,
    theme_set: Arc<ThemeSet>,
}

impl Default for Highlighter {
    fn default() -> Self {
        Self::new()
    }
}

impl Highlighter {
    pub fn new() -> Self {
        let assets = &*DEFAULT_ASSETS;
        Self {
            syntax_set: Arc::clone(&assets.0),
            theme_set: Arc::clone(&assets.1),
        }
    }

    pub fn available_themes(&self) -> Vec<String> {
        let mut themes: Vec<_> = self.theme_set.themes.keys().cloned().collect();
        themes.sort();
        themes
    }

    pub fn highlight(&self, path: &Path, lines: &[String], theme: &str) -> HighlightResult {
        let resolved_theme = self.resolve_theme(theme);
        let theme_name = resolved_theme.name.to_string();

        if let Some((syntax, language)) = self.syntax_for_path(path) {
            match self.highlight_with_syntax(lines, resolved_theme.theme, syntax) {
                Ok(highlighted) => HighlightResult {
                    lines: highlighted,
                    language: Some(language),
                    theme: theme_name,
                    mode: HighlightMode::Highlighted,
                },
                Err(err) => {
                    tracing::warn!(error = %err, path = %path.display(), "highlight failed");
                    HighlightResult::plain(lines.to_vec(), theme_name)
                }
            }
        } else {
            HighlightResult::plain(lines.to_vec(), theme_name)
        }
    }

    fn highlight_with_syntax(
        &self,
        lines: &[String],
        theme: &Theme,
        syntax: &SyntaxReference,
    ) -> Result<Vec<HighlightLine>> {
        let mut highlighter = HighlightLines::new(syntax, theme);
        let mut result = Vec::with_capacity(lines.len());
        for line in lines {
            let segments = highlighter.highlight_line(line, &self.syntax_set)?;
            let spans = segments
                .into_iter()
                .map(|(style, text)| HighlightSpan {
                    content: text.to_string(),
                    style: convert_style(style),
                })
                .collect();
            result.push(HighlightLine { spans });
        }
        Ok(result)
    }

    fn syntax_for_path(&self, path: &Path) -> Option<(&SyntaxReference, String)> {
        match self.syntax_set.find_syntax_for_file(path) {
            Ok(Some(syntax)) => Some((syntax, syntax.name.clone())),
            Ok(None) => None,
            Err(err) => {
                tracing::debug!(path = %path.display(), error = %err, "syntax lookup failed");
                None
            }
        }
    }

    fn resolve_theme<'a>(&'a self, requested: &'a str) -> ResolvedTheme<'a> {
        if let Some(theme) = self.theme_set.themes.get(requested) {
            return ResolvedTheme {
                name: Cow::Borrowed(requested),
                theme,
            };
        }

        if let Some(name) = self
            .theme_set
            .themes
            .keys()
            .find(|name| name.eq_ignore_ascii_case(requested))
            .cloned()
        {
            if let Some(theme) = self.theme_set.themes.get(&name) {
                return ResolvedTheme {
                    name: Cow::Owned(name),
                    theme,
                };
            }
        }

        let fallback_name = if self.theme_set.themes.contains_key(DEFAULT_THEME) {
            DEFAULT_THEME.to_string()
        } else {
            self.theme_set
                .themes
                .keys()
                .next()
                .cloned()
                .unwrap_or_else(|| DEFAULT_THEME.to_string())
        };

        let theme = self
            .theme_set
            .themes
            .get(&fallback_name)
            .expect("fallback theme must exist");

        tracing::warn!(
            requested,
            fallback = %fallback_name,
            "theme not found"
        );

        ResolvedTheme {
            name: Cow::Owned(fallback_name),
            theme,
        }
    }
}

#[derive(Debug, Clone)]
struct ResolvedTheme<'a> {
    name: Cow<'a, str>,
    theme: &'a Theme,
}

fn convert_style(style: SyntectStyle) -> HighlightStyle {
    let attributes = HighlightAttributes {
        bold: style.font_style.contains(FontStyle::BOLD),
        italic: style.font_style.contains(FontStyle::ITALIC),
        underline: style.font_style.contains(FontStyle::UNDERLINE),
    };

    HighlightStyle {
        foreground: convert_color(style.foreground),
        background: convert_color(style.background),
        attributes,
    }
}

fn convert_color(color: syntect::highlighting::Color) -> Option<RgbColor> {
    if color.a == 0 {
        None
    } else {
        Some(RgbColor {
            r: color.r,
            g: color.g,
            b: color.b,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn dracula_theme_is_available() {
        let highlighter = Highlighter::new();
        assert!(
            highlighter
                .available_themes()
                .iter()
                .any(|theme| theme.eq_ignore_ascii_case("dracula"))
        );
    }

    #[test]
    fn highlight_rust_file_produces_segments() -> Result<()> {
        let dir = tempdir()?;
        let file = dir.path().join("sample.rs");
        fs::write(&file, "fn main() { println!(\"hi\"); }\n")?;

        let highlighter = Highlighter::new();
        let lines = vec!["fn main() { println!(\"hi\"); }".to_string()];
        let result = highlighter.highlight(&file, &lines, "dracula");

        assert_eq!(result.lines.len(), 1);
        assert!(!result.lines[0].spans.is_empty());
        assert_eq!(result.mode, HighlightMode::Highlighted);
        assert_eq!(result.language.as_deref(), Some("Rust"));
        Ok(())
    }

    #[test]
    fn unknown_theme_falls_back() {
        let highlighter = Highlighter::new();
        let lines = vec!["plain text".to_string()];
        let file = Path::new("plain.txt");
        let result = highlighter.highlight(file, &lines, "not-a-theme");
        assert_eq!(result.mode, HighlightMode::Highlighted);
        assert_ne!(result.theme, "not-a-theme");
    }
}
