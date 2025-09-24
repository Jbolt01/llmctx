//! Preview service producing syntax highlighted, chunked views of files.

use std::borrow::Cow;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use crate::infra::config::Config;
use crate::infra::highlight::{HighlightResult, Highlighter};

/// Default continuation size when previewing large files if configuration is zero.
const DEFAULT_CHUNK_SIZE: usize = 200;

/// A continuation token used for loading more preview content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContinuationToken {
    pub start_line: usize,
}

/// Displayable preview output including metadata for the UI layer.
#[derive(Debug, Clone)]
pub struct PreviewSegment {
    pub path: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    pub highlighted: HighlightResult,
    pub truncated: bool,
    pub continuation: Option<ContinuationToken>,
    pub notice: Option<String>,
}

/// Service responsible for preparing preview data from files.
#[derive(Debug, Default)]
pub struct PreviewService {
    highlighter: Highlighter,
}

impl PreviewService {
    pub fn new() -> Self {
        Self {
            highlighter: Highlighter::new(),
        }
    }

    /// Load a preview segment for the provided path.
    pub fn preview(
        &self,
        path: &Path,
        range: Option<std::ops::Range<usize>>,
        config: &Config,
    ) -> Result<PreviewSegment> {
        if !path.exists() {
            return Err(anyhow!("file not found: {}", path.display()));
        }

        let start = range.as_ref().map_or(0, |r| r.start);

        if Self::is_binary(path)? {
            let message = format!(
                "Binary preview not available for {} (rendered as plain text).",
                path.display()
            );
            let theme = config.defaults.theme().to_string();
            let highlighted = HighlightResult::plain(Vec::new(), theme);
            return Ok(PreviewSegment {
                path: path.to_path_buf(),
                start_line: start + 1,
                end_line: start,
                truncated: false,
                continuation: None,
                notice: Some(message),
                highlighted,
            });
        }

        let configured_chunk = config.defaults.preview_max_lines();
        let chunk_size = if configured_chunk == 0 {
            DEFAULT_CHUNK_SIZE
        } else {
            configured_chunk
        };

        let limit = range
            .as_ref()
            .map(|r| r.end.saturating_sub(r.start))
            .filter(|len| *len > 0)
            .unwrap_or(chunk_size);

        let (lines, lossy, has_more) = Self::read_lines(path, start, limit)?;
        let mut notice = None;
        let theme_name = config.defaults.theme().to_string();

        let highlighted = if lossy {
            notice =
                Some("Preview rendered without syntax highlighting due to invalid UTF-8.".into());
            HighlightResult::plain(lines.clone(), theme_name)
        } else {
            self.highlighter
                .highlight(path, &lines, config.defaults.theme())
        };

        let end_line = start + lines.len();
        let continuation = has_more.then(|| ContinuationToken {
            start_line: start + lines.len(),
        });

        Ok(PreviewSegment {
            path: path.to_path_buf(),
            start_line: start + 1,
            end_line,
            truncated: has_more,
            highlighted,
            continuation,
            notice,
        })
    }

    /// Determine if the file should be treated as binary and skipped.
    fn is_binary(path: &Path) -> Result<bool> {
        let mut file = File::open(path)?;
        let mut buf = [0u8; 1024];
        let read = file.read(&mut buf)?;
        Ok(buf[..read].contains(&0))
    }

    fn read_lines(
        path: &Path,
        start: usize,
        max_lines: usize,
    ) -> Result<(Vec<String>, bool, bool)> {
        let file =
            File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
        let mut reader = BufReader::new(file);
        let mut raw = Vec::new();
        let mut lines = Vec::new();
        let mut lossy = false;
        let mut reached_eof = false;
        let mut index = 0;

        while index < start + max_lines {
            raw.clear();
            let bytes = reader.read_until(b'\n', &mut raw)?;
            if bytes == 0 {
                reached_eof = true;
                break;
            }

            if index >= start && lines.len() < max_lines {
                if raw.ends_with(&[b'\n']) {
                    raw.pop();
                    if raw.ends_with(&[b'\r']) {
                        raw.pop();
                    }
                }
                let text = String::from_utf8_lossy(&raw);
                if matches!(text, Cow::Owned(_)) {
                    lossy = true;
                }
                lines.push(text.into_owned());
            }

            index += 1;
            if lines.len() >= max_lines {
                break;
            }
        }

        let mut has_more = false;
        if !reached_eof && lines.len() == max_lines {
            let mut peek = [0u8; 1];
            has_more = reader.read(&mut peek)? > 0;
        }

        Ok((lines, lossy, has_more))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;
    use crate::infra::highlight::HighlightMode;

    fn config() -> Config {
        Config::default()
    }

    #[test]
    fn preview_small_file_returns_highlighted_segment() -> Result<()> {
        let dir = tempdir()?;
        let file = dir.path().join("hello.rs");
        std::fs::write(&file, "fn greet() { println!(\"hi\"); }\n")?;

        let service = PreviewService::new();
        let segment = service.preview(&file, None, &config())?;

        assert_eq!(segment.start_line, 1);
        assert_eq!(segment.end_line, 1);
        assert_eq!(segment.highlighted.mode, HighlightMode::Highlighted);
        assert!(segment.notice.is_none());
        Ok(())
    }

    #[test]
    fn preview_handles_range() -> Result<()> {
        let dir = tempdir()?;
        let file = dir.path().join("example.rs");
        let content = (0..500)
            .map(|i| format!("fn foo{i}() {{}}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&file, content)?;

        let service = PreviewService::new();
        let config = Config::default();
        let segment = service.preview(&file, Some(100..150), &config)?;

        assert_eq!(segment.start_line, 101);
        assert_eq!(segment.end_line, 150);
        assert!(segment.truncated);
        assert!(segment.continuation.is_some());
        Ok(())
    }

    #[test]
    fn binary_file_returns_notice() -> Result<()> {
        let dir = tempdir()?;
        let file = dir.path().join("data.bin");
        std::fs::write(&file, [0, 159, 146, 150])?;

        let service = PreviewService::new();
        let segment = service.preview(&file, None, &config())?;

        assert_eq!(segment.highlighted.mode, HighlightMode::Plain);
        assert!(segment.highlighted.lines.is_empty());
        assert!(
            segment
                .notice
                .as_ref()
                .is_some_and(|n| n.contains("Binary preview"))
        );
        assert!(!segment.truncated);
        Ok(())
    }

    #[test]
    fn lossy_content_falls_back_to_plain() -> Result<()> {
        let dir = tempdir()?;
        let file = dir.path().join("lossy.txt");
        let mut handle = File::create(&file)?;
        handle.write_all(b"hello\xffworld\n")?;
        drop(handle);

        let service = PreviewService::new();
        let segment = service.preview(&file, None, &config())?;

        assert_eq!(segment.highlighted.mode, HighlightMode::Plain);
        assert!(
            segment
                .notice
                .as_ref()
                .is_some_and(|n| n.contains("invalid UTF-8"))
        );
        assert_eq!(segment.end_line, 1);
        Ok(())
    }
}
