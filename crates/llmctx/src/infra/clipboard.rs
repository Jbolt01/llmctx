//! Clipboard integration utilities.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow};

/// Cross-platform clipboard helper with fallbacks for headless environments.
pub struct Clipboard {
    primary: Option<arboard::Clipboard>,
}

impl Clipboard {
    /// Attempt to initialize the system clipboard. When unavailable we fall back to shell-based
    /// clipboard utilities.
    pub fn new() -> Self {
        let primary = arboard::Clipboard::new().ok();
        Self { primary }
    }

    /// Copy text to the clipboard, falling back to platform-specific executables if needed.
    pub fn copy(&mut self, text: &str) -> Result<()> {
        if let Some(primary) = self.primary.as_mut()
            && primary.set_text(text.to_owned()).is_ok()
        {
            return Ok(());
        }

        self.primary = None;
        fallback_copy(text)
    }
}

impl Default for Clipboard {
    fn default() -> Self {
        Self::new()
    }
}

fn fallback_copy(text: &str) -> Result<()> {
    for command in fallback_commands() {
        if try_command_copy(command, text).is_ok() {
            return Ok(());
        }
    }

    Err(anyhow!(
        "failed to copy text to clipboard using available backends"
    ))
}

fn try_command_copy(command: &[&str], text: &str) -> Result<()> {
    let (program, args) = command
        .split_first()
        .context("clipboard command missing program")?;

    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn clipboard command: {program}"))?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(text.as_bytes())
            .context("failed to write clipboard contents")?;
    }

    let status = child
        .wait()
        .with_context(|| format!("clipboard command did not exit cleanly: {program}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("clipboard command exited with status {status}"))
    }
}

#[cfg(target_os = "macos")]
fn fallback_commands() -> Vec<&'static [&'static str]> {
    vec![&["pbcopy"]]
}

#[cfg(all(unix, not(target_os = "macos")))]
fn fallback_commands() -> Vec<&'static [&'static str]> {
    vec![&["xclip", "-selection", "clipboard"], &["wl-copy"]]
}

#[cfg(target_os = "windows")]
fn fallback_commands() -> Vec<&'static [&'static str]> {
    vec![&["powershell.exe", "-NoProfile", "-Command", "Set-Clipboard"]]
}

#[cfg(not(any(unix, target_os = "windows")))]
fn fallback_commands() -> Vec<&'static [&'static str]> {
    Vec::new()
}
