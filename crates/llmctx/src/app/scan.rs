//! Repository scanning services.

use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::{DirEntry, WalkBuilder, WalkState};
use time::OffsetDateTime;

use crate::infra::config::Config;

const LLMCTX_IGNORE: &str = ".llmctxignore";

/// Metadata describing a file discovered in the repository.
#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub path: PathBuf,
    pub display_path: String,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub modified: Option<OffsetDateTime>,
    pub language: Option<String>,
    pub skipped: Option<SkipReason>,
}

/// Reason for excluding or marking a file as skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    LargeFile,
    BinaryFile,
}

/// Result of scanning a repository root.
#[derive(Debug, Default)]
pub struct ScanResult {
    pub files: Vec<FileMetadata>,
    pub root: PathBuf,
}

/// Configuration inputs for the scanner.
#[derive(Debug, Clone)]
pub struct ScannerConfig {
    pub root: PathBuf,
    pub max_file_size: u64,
    pub config: Config,
}

impl ScannerConfig {
    pub fn from_root(root: PathBuf, config: Config) -> Self {
        Self {
            root,
            max_file_size: 1024 * 1024,
            config,
        }
    }

    pub fn with_max_file_size(mut self, bytes: u64) -> Self {
        self.max_file_size = bytes;
        self
    }
}

/// Scanner walking the repository respecting ignore rules and producing metadata.
#[derive(Debug, Default)]
pub struct Scanner;

impl Scanner {
    pub fn new() -> Self {
        Self
    }

    pub fn scan(&self, cfg: &ScannerConfig) -> Result<ScanResult> {
        let matcher = Arc::new(build_ignore_matcher(&cfg.root, cfg)?);
        let mut builder = WalkBuilder::new(&cfg.root);
        builder
            .git_ignore(true)
            .hidden(!cfg.config.defaults.show_hidden());

        let root = cfg.root.clone();
        builder.filter_entry({
            let matcher = matcher.clone();
            move |entry| {
                if entry.depth() == 0 {
                    return true;
                }
                let rel = entry.path().strip_prefix(&root).unwrap_or(entry.path());
                !matcher.should_skip(rel)
            }
        });

        let files = Mutex::new(Vec::new());
        let cfg_ref = Arc::new(cfg.clone());

        builder.build_parallel().run(|| {
            let files = &files;
            let cfg = cfg_ref.clone();
            Box::new(move |result| match result {
                Ok(entry) => {
                    if let Some(meta) = process_entry(&entry, &cfg)
                        && let Ok(mut guard) = files.lock()
                    {
                        guard.push(meta);
                    }
                    WalkState::Continue
                }
                Err(err) => {
                    tracing::warn!(error = %err, "scanner error");
                    WalkState::Continue
                }
            })
        });

        let mut files = files.into_inner().unwrap_or_default();
        files.sort_by(|a, b| a.display_path.cmp(&b.display_path));

        Ok(ScanResult {
            files,
            root: cfg.root.clone(),
        })
    }
}

fn process_entry(entry: &DirEntry, cfg: &ScannerConfig) -> Option<FileMetadata> {
    let path = entry.path();
    if path == cfg.root {
        return None;
    }

    let metadata = entry.metadata().ok()?;
    let is_dir = metadata.is_dir();
    let file_size = metadata.is_file().then_some(metadata.len());

    let mut skipped = None;
    if let Some(size) = file_size {
        if size > cfg.max_file_size {
            skipped = Some(SkipReason::LargeFile);
        } else if is_probably_binary(path) {
            skipped = Some(SkipReason::BinaryFile);
        }
    }

    let modified = metadata.modified().ok().map(OffsetDateTime::from);

    Some(FileMetadata {
        path: path.to_path_buf(),
        display_path: to_display_path(&cfg.root, path),
        is_dir,
        size: file_size,
        modified,
        language: if is_dir { None } else { guess_language(path) },
        skipped,
    })
}

fn to_display_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn guess_language(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
}

fn is_probably_binary(path: &Path) -> bool {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let mut buf = [0u8; 1024];
    match file.read(&mut buf) {
        Ok(0) => false,
        Ok(n) => {
            let slice = &buf[..n];
            slice.contains(&0) || std::str::from_utf8(slice).is_err()
        }
        Err(_) => false,
    }
}

#[derive(Debug, Clone)]
struct IgnoreMatcher {
    globs: Option<GlobSet>,
}

impl IgnoreMatcher {
    fn should_skip(&self, rel: &Path) -> bool {
        self.globs.as_ref().is_some_and(|set| set.is_match(rel))
    }
}

fn build_ignore_matcher(root: &Path, cfg: &ScannerConfig) -> Result<IgnoreMatcher> {
    let mut builder = GlobSetBuilder::new();

    for pattern in &cfg.config.ignore.paths {
        for expanded in expand_dir_pattern(pattern) {
            let glob = Glob::new(&expanded).context("invalid ignore path pattern")?;
            builder.add(glob);
        }
    }

    for glob in &cfg.config.ignore.globs {
        let glob = Glob::new(glob).context("invalid ignore glob")?;
        builder.add(glob);
    }

    for pattern in load_llmctxignore(root)? {
        for expanded in expand_dir_pattern(&pattern) {
            let glob = Glob::new(&expanded).context("invalid .llmctxignore pattern")?;
            builder.add(glob);
        }
    }

    // Always ignore the ignore file itself.
    builder.add(Glob::new(LLMCTX_IGNORE)?);

    let globs = builder.build().context("failed to build ignore matcher")?;

    Ok(IgnoreMatcher { globs: Some(globs) })
}

fn expand_dir_pattern(raw: &str) -> Vec<String> {
    let trimmed = raw.trim().trim_matches('/');
    if trimmed.is_empty() {
        return Vec::new();
    }
    vec![
        trimmed.to_owned(),
        format!("{trimmed}/**"),
        format!("**/{trimmed}"),
        format!("**/{trimmed}/**"),
    ]
}

fn load_llmctxignore(root: &Path) -> Result<Vec<String>> {
    let path = root.join(LLMCTX_IGNORE);
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(&path).with_context(|| format!("failed to open {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut patterns = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        patterns.push(trimmed.to_owned());
    }
    Ok(patterns)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn build_config() -> Config {
        Config::default()
    }

    #[test]
    fn respects_ignore_paths_and_globs() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path();

        fs::create_dir_all(root.join("src"))?;
        fs::create_dir_all(root.join("skipme"))?;
        fs::create_dir_all(root.join("target"))?;
        fs::write(root.join("src/lib.rs"), b"fn lib() {}")?;
        fs::write(root.join("skipme/file.txt"), b"ignored")?;
        fs::write(root.join("Cargo.lock"), b"lock")?;

        let mut config = build_config();
        config.ignore.paths.push("skipme/".into());
        config.ignore.globs.push("*.lock".into());

        let scanner_cfg = ScannerConfig::from_root(root.to_path_buf(), config);
        let result = Scanner::new().scan(&scanner_cfg)?;

        let paths: Vec<_> = result
            .files
            .iter()
            .map(|f| f.display_path.clone())
            .collect();

        assert!(paths.contains(&"src/lib.rs".to_string()));
        assert!(!paths.iter().any(|p| p.contains("skipme")));
        assert!(!paths.iter().any(|p| p.ends_with("Cargo.lock")));
        Ok(())
    }

    #[test]
    fn marks_large_and_binary_files() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path();

        fs::write(root.join("large.bin"), vec![0u8; 4096])?;
        fs::write(root.join("binary.dat"), b"abc\0def")?;
        fs::write(root.join("text.txt"), b"hello world")?;

        let config = build_config();
        let scanner_cfg =
            ScannerConfig::from_root(root.to_path_buf(), config).with_max_file_size(1024);

        let result = Scanner::new().scan(&scanner_cfg)?;

        let large = result
            .files
            .iter()
            .find(|f| f.display_path == "large.bin")
            .expect("large.bin present");
        assert_eq!(large.skipped, Some(SkipReason::LargeFile));

        let binary = result
            .files
            .iter()
            .find(|f| f.display_path == "binary.dat")
            .expect("binary.dat present");
        assert_eq!(binary.skipped, Some(SkipReason::BinaryFile));

        let text = result
            .files
            .iter()
            .find(|f| f.display_path == "text.txt")
            .expect("text.txt present");
        assert_eq!(text.skipped, None);
        Ok(())
    }

    #[test]
    fn respects_llmctxignore() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path();

        fs::create_dir_all(root.join("generated"))?;
        fs::create_dir_all(root.join("src"))?;
        fs::write(root.join("generated/output.txt"), b"not included")?;
        fs::write(root.join("src/main.rs"), b"fn main() {}")?;
        fs::write(root.join(LLMCTX_IGNORE), "generated/\n")?;

        let config = build_config();
        let scanner_cfg = ScannerConfig::from_root(root.to_path_buf(), config);

        let result = Scanner::new().scan(&scanner_cfg)?;
        let paths: Vec<_> = result
            .files
            .iter()
            .map(|f| f.display_path.as_str())
            .collect();

        assert!(paths.contains(&"src/main.rs"));
        assert!(!paths.iter().any(|p| p.starts_with("generated")));
        Ok(())
    }
}
