use std::fs;
use std::io::Write;
use std::path::PathBuf;

use llmctx::app::export::{ExportFormat, ExportOptions, Exporter};
use llmctx::app::selection::SelectionManager;
use llmctx::app::tokens::TokenEstimator;
use llmctx::infra::config::Config;
use tempfile::NamedTempFile;

fn create_temp_file(contents: &str) -> (PathBuf, NamedTempFile) {
    let mut file = NamedTempFile::new().expect("temp file");
    write!(file, "{}", contents).expect("write contents");
    (file.path().to_path_buf(), file)
}

#[test]
fn exports_markdown_bundle_with_line_numbers() {
    let (path, _file) = create_temp_file("fn main() {}\n// comment\nprintln!(\"done\");\n");

    let mut manager = SelectionManager::new();
    manager.add_selection(&path, Some((1, 2)), Some("core entry point".into()));

    let config = Config::default();
    let estimator = TokenEstimator::from_config(&config);
    let summary = manager.summarize_tokens(&estimator).unwrap();

    let mut options = ExportOptions::from_config(&config);
    let temp_dir = tempfile::tempdir().unwrap();
    let output_path = temp_dir.path().join("context.md");
    options.output_path = Some(output_path.clone());

    let exporter = Exporter::new().unwrap();
    let bundle = manager.to_bundle();
    let result = exporter.export(&bundle, summary.as_ref(), &options).unwrap();

    assert!(result.rendered.contains("Curated Context"));
    assert!(result.rendered.contains("Lines 1-2"));
    assert!(result.rendered.contains("fn main() {}"));
    assert!(result.rendered.contains("core entry point"));

    let written = fs::read_to_string(output_path).unwrap();
    assert!(written.contains("Curated Context"));
}

#[test]
fn exports_plain_text_when_requested() {
    let (path, _file) = create_temp_file("alpha\nbeta\n");

    let mut manager = SelectionManager::new();
    manager.add_selection(&path, None, None);

    let config = Config::default();
    let estimator = TokenEstimator::from_config(&config);
    let summary = manager.summarize_tokens(&estimator).unwrap();

    let mut options = ExportOptions::from_config(&config);
    options.format = ExportFormat::Plain;
    options.template = "plain_text".into();

    let exporter = Exporter::new().unwrap();
    let bundle = manager.to_bundle();
    let result = exporter.export(&bundle, summary.as_ref(), &options).unwrap();

    assert!(result.rendered.contains("Curated context generated"));
    assert!(result.rendered.contains("alpha"));
    assert!(result.rendered.contains("beta"));
}
