//! Export bundle handling.

use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow};
use clap::ValueEnum;
use minijinja::Environment;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::app::tokens::BundleTokenSummary;
use crate::domain::model::{ContextBundle, SelectionItem};
use crate::infra::clipboard::Clipboard;
use crate::infra::config::Config;
use crate::infra::git::{self, GitMetadata};

/// Supported export formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[value(rename_all = "kebab-case")]
pub enum ExportFormat {
    /// Markdown document with fenced code blocks.
    Markdown,
    /// Plain text report.
    Plain,
}

impl ExportFormat {
    /// Return a stable identifier for templates and configuration.
    pub fn as_str(&self) -> &'static str {
        match self {
            ExportFormat::Markdown => "markdown",
            ExportFormat::Plain => "plain",
        }
    }

    /// Recommended file extension for the format.
    pub fn extension(&self) -> &'static str {
        match self {
            ExportFormat::Markdown => "md",
            ExportFormat::Plain => "txt",
        }
    }
}

impl FromStr for ExportFormat {
    type Err = ExportFormatParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "markdown" | "md" | "commonmark" => Ok(ExportFormat::Markdown),
            "plain" | "text" | "txt" => Ok(ExportFormat::Plain),
            other => Err(ExportFormatParseError::UnknownFormat(other.to_string())),
        }
    }
}

/// Error returned when parsing an [`ExportFormat`] fails.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ExportFormatParseError {
    #[error("unknown export format '{0}'")]
    UnknownFormat(String),
}

/// Runtime options controlling export behavior.
#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub format: ExportFormat,
    pub template: String,
    pub include_line_numbers: bool,
    pub include_git_metadata: bool,
    pub output_path: Option<PathBuf>,
    pub copy_to_clipboard: bool,
}

impl ExportOptions {
    /// Build options from configuration defaults.
    pub fn from_config(config: &Config) -> Self {
        let format = <ExportFormat as std::str::FromStr>::from_str(config.defaults.export_format())
            .unwrap_or(ExportFormat::Markdown);
        Self {
            format,
            template: config.export.template(),
            include_line_numbers: config.export.include_line_numbers(),
            include_git_metadata: config.export.include_git_metadata(),
            output_path: None,
            copy_to_clipboard: false,
        }
    }
}

/// Result of an export operation.
#[derive(Debug, Clone)]
pub struct ExportResult {
    pub rendered: String,
    pub output_path: Option<PathBuf>,
    pub copied_to_clipboard: bool,
}

/// Responsible for rendering bundles and writing artifacts.
pub struct Exporter {
    env: Environment<'static>,
    clipboard: Mutex<Clipboard>,
}

impl Exporter {
    /// Create a new exporter with built-in templates loaded.
    pub fn new() -> Result<Self> {
        Ok(Self {
            env: default_environment()?,
            clipboard: Mutex::new(Clipboard::new()),
        })
    }

    /// Render the provided bundle into a string using the supplied options.
    pub fn render_bundle(
        &self,
        bundle: &ContextBundle,
        summary: Option<&BundleTokenSummary>,
        options: &ExportOptions,
    ) -> Result<String> {
        let git_metadata = if options.include_git_metadata {
            bundle
                .items
                .first()
                .and_then(|item| git::metadata_for_path(&item.path))
        } else {
            None
        };

        let context = build_template_context(bundle, summary, options, git_metadata)?;
        self.render_with_template(&context, &options.template)
    }

    /// Render the bundle and persist/copy outputs based on options.
    pub fn export(
        &self,
        bundle: &ContextBundle,
        summary: Option<&BundleTokenSummary>,
        options: &ExportOptions,
    ) -> Result<ExportResult> {
        let rendered = self.render_bundle(bundle, summary, options)?;

        if let Some(path) = &options.output_path {
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent).with_context(|| {
                    format!("failed to create export directory: {}", parent.display())
                })?;
            }
            fs::write(path, &rendered)
                .with_context(|| format!("failed to write export output to {}", path.display()))?;
        }

        if options.copy_to_clipboard {
            self.clipboard
                .lock()
                .unwrap()
                .copy(&rendered)
                .context("failed to copy export to clipboard")?;
        }

        Ok(ExportResult {
            rendered,
            output_path: options.output_path.clone(),
            copied_to_clipboard: options.copy_to_clipboard,
        })
    }

    fn render_with_template(
        &self,
        context: &TemplateContext,
        template_name: &str,
    ) -> Result<String> {
        if let Ok(template) = self.env.get_template(template_name) {
            return template
                .render(context)
                .map_err(|err| anyhow!("failed to render template '{template_name}': {err}"));
        }

        let template_path = Path::new(template_name);
        if template_path.exists() {
            let source = fs::read_to_string(template_path).with_context(|| {
                format!(
                    "failed to load template from path {}",
                    template_path.display()
                )
            })?;
            let mut env = Environment::new();
            env.set_trim_blocks(true);
            env.set_lstrip_blocks(true);
            env.add_template("external", &source)
                .map_err(|err| anyhow!("invalid template '{}': {err}", template_name))?;
            return env
                .get_template("external")
                .unwrap()
                .render(context)
                .map_err(|err| anyhow!("failed to render template '{template_name}': {err}"));
        }

        Err(anyhow!(
            "template '{}' not found (built-in or filesystem)",
            template_name
        ))
    }
}

fn default_environment() -> Result<Environment<'static>> {
    let mut env = Environment::new();
    env.set_trim_blocks(true);
    env.set_lstrip_blocks(true);
    env.add_template("concise_context", DEFAULT_MARKDOWN_TEMPLATE)
        .map_err(|err| anyhow!("failed to register default markdown template: {err}"))?;
    env.add_template("plain_text", DEFAULT_PLAIN_TEMPLATE)
        .map_err(|err| anyhow!("failed to register default plain template: {err}"))?;
    Ok(env)
}

fn build_template_context(
    bundle: &ContextBundle,
    summary: Option<&BundleTokenSummary>,
    options: &ExportOptions,
    git_metadata: Option<GitMetadata>,
) -> Result<TemplateContext> {
    let generated_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .context("failed to format export timestamp")?;

    let mut selections = Vec::with_capacity(bundle.items.len());
    for (index, item) in bundle.items.iter().enumerate() {
        let summary_item = summary.and_then(|summary| summary.items.get(index));
        let extracted = extract_selection_contents(item, options.include_line_numbers)?;
        selections.push(TemplateSelection {
            path: item.path.display().to_string(),
            display_path: display_path(item, git_metadata.as_ref()),
            range: item.range.map(|(start, end)| SelectionRange { start, end }),
            start_line: extracted.start_line,
            end_line: extracted.end_line,
            contents: extracted.contents,
            note: item.note.clone(),
            tokens: summary_item.map(|entry| entry.tokens),
            characters: summary_item
                .map(|entry| entry.characters)
                .or(Some(extracted.character_count)),
        });
    }

    let tokens = summary.map(|summary| TemplateTokenSummary {
        model: summary.model.as_str().to_string(),
        token_budget: summary.token_budget,
        total_tokens: summary.total_tokens,
        total_characters: summary.total_characters,
    });

    Ok(TemplateContext {
        generated_at,
        format: options.format.as_str().to_string(),
        model: bundle.model.clone(),
        selections,
        tokens,
        git: git_metadata,
    })
}

fn display_path(item: &SelectionItem, git_metadata: Option<&GitMetadata>) -> String {
    let path = &item.path;
    if let Some(metadata) = git_metadata
        && let Ok(relative) = path.strip_prefix(&metadata.root)
    {
        return relative.display().to_string();
    }

    if let Ok(cwd) = std::env::current_dir()
        && let Ok(relative) = path.strip_prefix(&cwd)
    {
        return relative.display().to_string();
    }

    path.display().to_string()
}

fn extract_selection_contents(
    item: &SelectionItem,
    include_line_numbers: bool,
) -> Result<SelectionExtraction> {
    let contents = fs::read_to_string(&item.path).with_context(|| {
        format!(
            "failed to read selection contents from {}",
            item.path.display()
        )
    })?;

    let lines: Vec<&str> = contents.lines().collect();
    let total_lines = lines.len();

    let (raw_start, raw_end) = item.range.unwrap_or((1, total_lines.max(1)));
    let start = raw_start.max(1);
    let end = raw_end.max(start);
    let available_end = if total_lines == 0 { 0 } else { total_lines };
    let clamped_start = if available_end == 0 {
        start
    } else {
        start.min(available_end)
    };
    let clamped_end = if available_end == 0 {
        end
    } else {
        end.min(available_end)
    };
    let display_end = clamped_end.max(clamped_start);
    let width = display_end.max(1).to_string().len();

    let mut extracted_lines = Vec::new();
    for (idx, line) in contents.lines().enumerate() {
        let line_no = idx + 1;
        if line_no < clamped_start || line_no > clamped_end {
            continue;
        }
        if include_line_numbers {
            extracted_lines.push(format!("{line_no:>width$} │ {line}", width = width));
        } else {
            extracted_lines.push(line.to_string());
        }
    }

    let joined = extracted_lines.join("\n");
    Ok(SelectionExtraction {
        contents: joined.clone(),
        start_line: Some(clamped_start),
        end_line: Some(clamped_end),
        character_count: joined.chars().count(),
    })
}

#[derive(Serialize)]
struct TemplateContext {
    generated_at: String,
    format: String,
    model: Option<String>,
    selections: Vec<TemplateSelection>,
    tokens: Option<TemplateTokenSummary>,
    git: Option<GitMetadata>,
}

#[derive(Serialize)]
struct TemplateSelection {
    path: String,
    display_path: String,
    range: Option<SelectionRange>,
    start_line: Option<usize>,
    end_line: Option<usize>,
    contents: String,
    note: Option<String>,
    tokens: Option<usize>,
    characters: Option<usize>,
}

#[derive(Serialize)]
struct SelectionRange {
    start: usize,
    end: usize,
}

#[derive(Serialize)]
struct TemplateTokenSummary {
    model: String,
    token_budget: u32,
    total_tokens: usize,
    total_characters: usize,
}

struct SelectionExtraction {
    contents: String,
    start_line: Option<usize>,
    end_line: Option<usize>,
    character_count: usize,
}

const DEFAULT_MARKDOWN_TEMPLATE: &str = r#"# Curated Context

Generated at: {{ generated_at }}

{% if tokens %}
## Token Summary
- Model: {{ tokens.model }}
- Usage: {{ tokens.total_tokens }} / {{ tokens.token_budget }} tokens
- Characters: {{ tokens.total_characters }}
{% endif %}

{% if git %}
## Repository
- Root: {{ git.root }}
{% if git.branch %}- Branch: {{ git.branch }}{% endif %}
{% if git.commit %}- Commit: {{ git.commit }}{% endif %}
{% endif %}

{% for selection in selections %}
## {{ loop.index }}. {{ selection.display_path }}
{% if selection.range %}_Lines {{ selection.range.start }}-{{ selection.range.end }}_{% endif %}
{% if selection.note %}> {{ selection.note }}

{% endif %}
```text
{{ selection.contents }}
```

{% if selection.tokens %}- Tokens: {{ selection.tokens }}{% endif %}
{% if selection.characters %}- Characters: {{ selection.characters }}{% endif %}

{% endfor %}
"#;

const DEFAULT_PLAIN_TEMPLATE: &str = r#"Curated context generated at {{ generated_at }}

{% if tokens %}Token summary: model {{ tokens.model }}, {{ tokens.total_tokens }}/{{ tokens.token_budget }} tokens, {{ tokens.total_characters }} characters.
{% endif %}
{% if git %}Repository: {{ git.root }}{% if git.branch %} (branch {{ git.branch }}){% endif %}{% if git.commit %} commit {{ git.commit }}{% endif %}.
{% endif %}

{% for selection in selections %}
-- {{ loop.index }}. {{ selection.display_path }}{% if selection.range %} (lines {{ selection.range.start }}-{{ selection.range.end }}){% endif %}
{% if selection.note %}Note: {{ selection.note }}
{% endif %}
{{ selection.contents }}

{% if selection.tokens %}Tokens: {{ selection.tokens }}{% endif %}{% if selection.characters %} Characters: {{ selection.characters }}{% endif %}

{% endfor %}
"#;
