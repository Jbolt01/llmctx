use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use clap::{Args as ClapArgs, Parser, Subcommand, ValueHint};

use llmctx::app::export::{ExportFormat, ExportOptions, Exporter};
use llmctx::app::selection::SelectionManager;
use llmctx::app::tokens::TokenEstimator;
use llmctx::infra::config::Config;

fn main() -> Result<()> {
    llmctx::init();

    let cli = Cli::parse();
    match cli.command.unwrap_or_default() {
        Command::Export(args) => run_export(args),
        Command::Tui => run_tui(),
    }
}

fn run_tui() -> Result<()> {
    let mut app = llmctx::ui::app::UiApp::default();
    app.run()
}

fn run_export(args: ExportArgs) -> Result<()> {
    let mut config = Config::load()?;
    if let Some(path) = &args.config {
        let overlay = Config::load_from_path(path)
            .with_context(|| format!("failed to load configuration from {}", path.display()))?;
        config = config.merge_with(overlay);
    }

    let selections = build_selection_manager(&args)?;
    if selections.is_empty() {
        return Err(anyhow!("at least one selection must be provided"));
    }

    let mut manager = SelectionManager::new();
    let model = args
        .model
        .unwrap_or_else(|| config.defaults.model().to_string());
    manager.set_model(model);
    for selection in selections {
        manager.add_selection(selection.path, selection.range, selection.note);
    }

    let estimator = TokenEstimator::from_config(&config);
    let summary = manager.summarize_tokens(&estimator)?;

    let mut options = ExportOptions::from_config(&config);
    if let Some(format) = args.format {
        options.format = format;
    }
    if let Some(template) = args.template {
        options.template = template;
    }
    options.output_path = args.output.clone();
    options.copy_to_clipboard = args.copy;

    let exporter = Exporter::new()?;
    let bundle = manager.to_bundle();
    let result = exporter.export(&bundle, summary.as_ref(), &options)?;

    if args.print {
        println!("{}", result.rendered);
    }

    Ok(())
}

fn build_selection_manager(args: &ExportArgs) -> Result<Vec<SelectionSpec>> {
    let mut selections = Vec::new();

    for spec in &args.selections {
        selections.push(spec.clone());
    }

    for path in &args.paths {
        selections.push(SelectionSpec {
            path: path.clone(),
            range: None,
            note: None,
        });
    }

    Ok(selections)
}

#[derive(Parser)]
#[command(
    name = "llmctx",
    version,
    about = "Curate and export context for LLM prompts"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Default)]
enum Command {
    /// Launch the interactive terminal UI.
    #[default]
    Tui,
    /// Export selections without launching the UI.
    Export(ExportArgs),
}

#[derive(ClapArgs, Debug, Clone)]
struct ExportArgs {
    /// Additional configuration file layered on top of defaults.
    #[arg(long, value_hint = ValueHint::FilePath)]
    config: Option<PathBuf>,
    /// Override the export format (markdown/plain).
    #[arg(long)]
    format: Option<ExportFormat>,
    /// Override the template name or path.
    #[arg(long)]
    template: Option<String>,
    /// Path to write the export contents to.
    #[arg(long, value_hint = ValueHint::FilePath)]
    output: Option<PathBuf>,
    /// Copy the rendered export to the system clipboard.
    #[arg(long)]
    copy: bool,
    /// Print the rendered output to stdout in addition to other actions.
    #[arg(long)]
    print: bool,
    /// Override the token model used for estimation.
    #[arg(long)]
    model: Option<String>,
    /// Explicit selections with optional ranges and notes (path[:start-end][#note]).
    #[arg(long = "select", value_name = "SPEC", value_parser = parse_selection_spec)]
    selections: Vec<SelectionSpec>,
    /// Entire file selections provided as positional arguments.
    #[arg(value_name = "PATH", value_hint = ValueHint::FilePath)]
    paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
struct SelectionSpec {
    path: PathBuf,
    range: Option<(usize, usize)>,
    note: Option<String>,
}

fn parse_selection_spec(value: &str) -> Result<SelectionSpec, String> {
    let (target, note) = match value.split_once('#') {
        Some((target, note)) => (target.trim(), clean_note_string(note)),
        None => (value.trim(), None),
    };

    if target.is_empty() {
        return Err("selection specification is empty".to_string());
    }

    let mut path_part = target;
    let mut range = None;

    if let Some(colon_idx) = target.rfind(':') {
        let (candidate_path, candidate_range) = target.split_at(colon_idx);
        if let Some(parsed_range) = parse_range(&candidate_range[1..]) {
            path_part = candidate_path;
            range = Some(parsed_range);
        }
    }

    if path_part.is_empty() {
        return Err("selection path is empty".to_string());
    }

    Ok(SelectionSpec {
        path: PathBuf::from(path_part),
        range,
        note,
    })
}

fn parse_range(spec: &str) -> Option<(usize, usize)> {
    let (start, end) = spec.split_once('-')?;
    let start = start.trim().parse::<usize>().ok()?;
    let end = end.trim().parse::<usize>().ok()?;
    Some((start, end))
}

fn clean_note_string(note: &str) -> Option<String> {
    let trimmed = note.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
