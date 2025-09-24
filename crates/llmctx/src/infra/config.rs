//! Configuration management utilities.

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use dirs_next::config_dir;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

static DEFAULT_CONFIG: Lazy<&'static str> =
    Lazy::new(|| include_str!("../../assets/default-config.toml"));
static DEFAULT_WORKSPACE_CONFIG_PATH: &str = ".llmctx/config.toml";

/// Layered configuration loaded from defaults, user, workspace, and env.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub ignore: Ignore,
    #[serde(default)]
    pub export: Export,
    #[serde(default)]
    pub keybindings: Keybindings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Defaults {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    export_format: Option<String>,
    #[serde(default)]
    token_budget: Option<u32>,
    #[serde(default)]
    theme: Option<String>,
    #[serde(default)]
    preview_max_lines: Option<usize>,
    #[serde(default)]
    show_hidden: Option<bool>,
}

impl Defaults {
    fn default_model() -> &'static str {
        "openai:gpt-4o-mini"
    }

    fn default_export_format() -> &'static str {
        "markdown"
    }

    fn default_token_budget() -> u32 {
        120_000
    }

    fn default_theme() -> &'static str {
        "dracula"
    }

    fn default_preview_max_lines() -> usize {
        400
    }

    pub fn model(&self) -> &str {
        self.model.as_deref().unwrap_or(Self::default_model())
    }

    pub fn export_format(&self) -> &str {
        self.export_format
            .as_deref()
            .unwrap_or(Self::default_export_format())
    }

    pub fn token_budget(&self) -> u32 {
        self.token_budget.unwrap_or_else(Self::default_token_budget)
    }

    pub fn theme(&self) -> &str {
        self.theme.as_deref().unwrap_or(Self::default_theme())
    }

    pub fn preview_max_lines(&self) -> usize {
        self.preview_max_lines
            .unwrap_or_else(Self::default_preview_max_lines)
    }

    pub fn show_hidden(&self) -> bool {
        self.show_hidden.unwrap_or(false)
    }
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            model: Some(Self::default_model().to_owned()),
            export_format: Some(Self::default_export_format().to_owned()),
            token_budget: Some(Self::default_token_budget()),
            theme: Some(Self::default_theme().to_owned()),
            preview_max_lines: Some(Self::default_preview_max_lines()),
            show_hidden: Some(false),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ignore {
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub globs: Vec<String>,
}

impl Default for Ignore {
    fn default() -> Self {
        Self {
            paths: vec![
                "target/".into(),
                "node_modules/".into(),
                "dist/".into(),
                ".git/".into(),
            ],
            globs: vec!["*.min.js".into(), "*.lock".into()],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Export {
    #[serde(default)]
    include_git_metadata: Option<bool>,
    #[serde(default)]
    include_line_numbers: Option<bool>,
    #[serde(default)]
    template: Option<String>,
}

impl Export {
    fn default_include_git_metadata() -> bool {
        true
    }

    fn default_include_line_numbers() -> bool {
        true
    }

    fn default_template() -> &'static str {
        "concise_context"
    }

    pub fn include_git_metadata(&self) -> bool {
        self.include_git_metadata
            .unwrap_or_else(Self::default_include_git_metadata)
    }

    pub fn include_line_numbers(&self) -> bool {
        self.include_line_numbers
            .unwrap_or_else(Self::default_include_line_numbers)
    }

    pub fn template(&self) -> String {
        self.template
            .clone()
            .unwrap_or_else(|| Self::default_template().to_owned())
    }
}

impl Default for Export {
    fn default() -> Self {
        Self {
            include_git_metadata: Some(Self::default_include_git_metadata()),
            include_line_numbers: Some(Self::default_include_line_numbers()),
            template: Some(Self::default_template().to_owned()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keybindings {
    #[serde(default = "Keybindings::default_up")]
    pub up: String,
    #[serde(default = "Keybindings::default_down")]
    pub down: String,
    #[serde(default = "Keybindings::default_select")]
    pub select: String,
    #[serde(default = "Keybindings::default_export")]
    pub export: String,
}

impl Keybindings {
    fn default_up() -> String {
        "k".into()
    }

    fn default_down() -> String {
        "j".into()
    }

    fn default_select() -> String {
        "space".into()
    }

    fn default_export() -> String {
        "ctrl+e".into()
    }
}

impl Default for Keybindings {
    fn default() -> Self {
        Self {
            up: Self::default_up(),
            down: Self::default_down(),
            select: Self::default_select(),
            export: Self::default_export(),
        }
    }
}

/// Environment overrides for critical settings.
#[derive(Debug, Default, Clone)]
pub struct EnvOverrides {
    model: Option<String>,
    export_format: Option<String>,
}

impl EnvOverrides {
    fn from_env() -> Self {
        Self {
            model: env::var("LLMCTX_MODEL").ok(),
            export_format: env::var("LLMCTX_EXPORT_FORMAT").ok(),
        }
    }

    #[cfg(test)]
    fn for_tests(model: &str, export_format: &str) -> Self {
        Self {
            model: Some(model.to_owned()),
            export_format: Some(export_format.to_owned()),
        }
    }
}

impl Config {
    /// Load configuration from defaults, user/global config, workspace config, and env overrides.
    pub fn load() -> Result<Self> {
        let env = EnvOverrides::from_env();
        let global = global_config_path();
        let workspace = workspace_config_path()?;
        Self::load_with_layers(global, workspace, env)
    }

    /// Load configuration from a single explicit path layered on top of defaults.
    pub fn load_from_path(path: &Path) -> Result<Self> {
        let defaults = Self::from_str(&DEFAULT_CONFIG)?;
        let explicit = Self::from_file(path)?;
        Ok(defaults.merge(explicit))
    }

    /// Merge another configuration on top of this instance, returning the combined result.
    pub fn merge_with(self, other: Config) -> Config {
        self.merge(other)
    }

    fn load_with_layers(
        global: Option<PathBuf>,
        workspace: Option<PathBuf>,
        env_overrides: EnvOverrides,
    ) -> Result<Self> {
        let mut layers: Vec<Config> = Vec::new();

        layers.push(Self::from_str(&DEFAULT_CONFIG)?);

        if let Some(global_path) = global.filter(|path| path.exists()) {
            layers.push(Self::from_file(&global_path)?);
        }

        if let Some(workspace_path) = workspace.filter(|path| path.exists()) {
            layers.push(Self::from_file(&workspace_path)?);
        }

        let merged = layers.into_iter().reduce(Config::merge).unwrap_or_default();
        Ok(apply_env_overrides(merged, env_overrides))
    }

    fn from_file(path: &Path) -> Result<Self> {
        let data = fs::read_to_string(path)
            .with_context(|| format!("failed to read config file: {}", path.display()))?;
        Self::from_str(&data)
    }

    fn from_str(contents: &str) -> Result<Self> {
        let config: Config =
            toml::from_str(contents).with_context(|| "failed to parse TOML config".to_string())?;
        Ok(config)
    }

    fn merge(self, other: Self) -> Self {
        Self {
            defaults: merge_defaults(self.defaults, other.defaults),
            ignore: merge_ignore(self.ignore, other.ignore),
            export: merge_export(self.export, other.export),
            keybindings: merge_keybindings(self.keybindings, other.keybindings),
        }
    }
}

fn merge_defaults(mut base: Defaults, overlay: Defaults) -> Defaults {
    if overlay.model.is_some() {
        base.model = overlay.model;
    }
    if overlay.export_format.is_some() {
        base.export_format = overlay.export_format;
    }
    if overlay.token_budget.is_some() {
        base.token_budget = overlay.token_budget;
    }
    if overlay.theme.is_some() {
        base.theme = overlay.theme;
    }
    if overlay.preview_max_lines.is_some() {
        base.preview_max_lines = overlay.preview_max_lines;
    }
    if overlay.show_hidden.is_some() {
        base.show_hidden = overlay.show_hidden;
    }
    base
}

fn merge_ignore(base: Ignore, overlay: Ignore) -> Ignore {
    let mut paths: BTreeSet<String> = base.paths.into_iter().collect();
    paths.extend(overlay.paths);

    let mut globs: BTreeSet<String> = base.globs.into_iter().collect();
    globs.extend(overlay.globs);

    Ignore {
        paths: paths.into_iter().collect(),
        globs: globs.into_iter().collect(),
    }
}

fn merge_export(mut base: Export, overlay: Export) -> Export {
    if let Some(value) = overlay.include_git_metadata {
        base.include_git_metadata = Some(value);
    }
    if let Some(value) = overlay.include_line_numbers {
        base.include_line_numbers = Some(value);
    }
    if let Some(value) = overlay.template {
        base.template = Some(value);
    }
    base
}

fn merge_keybindings(base: Keybindings, overlay: Keybindings) -> Keybindings {
    Keybindings {
        up: choose_keybinding(base.up, overlay.up, Keybindings::default_up),
        down: choose_keybinding(base.down, overlay.down, Keybindings::default_down),
        select: choose_keybinding(base.select, overlay.select, Keybindings::default_select),
        export: choose_keybinding(base.export, overlay.export, Keybindings::default_export),
    }
}

fn choose_keybinding(base: String, overlay: String, default_fn: fn() -> String) -> String {
    if overlay != default_fn() {
        overlay
    } else {
        base
    }
}

fn global_config_path() -> Option<PathBuf> {
    config_dir().map(|base| base.join("llmctx/config.toml"))
}

fn workspace_config_path() -> Result<Option<PathBuf>> {
    let cwd = env::current_dir()?;
    let root = find_repo_root(&cwd).unwrap_or(cwd);
    Ok(Some(root.join(DEFAULT_WORKSPACE_CONFIG_PATH)))
}

fn find_repo_root(start: &Path) -> Option<PathBuf> {
    let mut current = start;
    loop {
        if current.join(".git").exists() {
            return Some(current.to_path_buf());
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => return None,
        }
    }
}

fn apply_env_overrides(mut config: Config, env: EnvOverrides) -> Config {
    if let Some(model) = env.model {
        config.defaults.model = Some(model);
    }
    if let Some(export_format) = env.export_format {
        config.defaults.export_format = Some(export_format);
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_uses_defaults_when_no_files() {
        let config = Config::load_with_layers(None, None, EnvOverrides::default())
            .expect("load default config");
        assert_eq!(config.defaults.model(), "openai:gpt-4o-mini");
        assert!(config.ignore.paths.contains(&"target/".into()));
    }

    #[test]
    fn merge_global_and_workspace() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let global = temp.path().join("config.toml");
        fs::write(
            &global,
            r#"
[defaults]
model = "anthropic:claude"
[ignore]
paths = ["generated/"]
"#,
        )?;

        let workspace_dir = temp.path().join("repo");
        fs::create_dir_all(workspace_dir.join(".llmctx"))?;
        fs::create_dir_all(workspace_dir.join(".git"))?;
        fs::write(
            workspace_dir.join(".llmctx/config.toml"),
            r#"
[defaults]
export_format = "json"
[ignore]
globs = ["*.cache"]
"#,
        )?;

        let global_path = Some(global);
        let workspace_path = Some(workspace_dir.join(".llmctx/config.toml"));

        let config =
            Config::load_with_layers(global_path, workspace_path, EnvOverrides::default())?;

        assert_eq!(config.defaults.model(), "anthropic:claude");
        assert_eq!(config.defaults.export_format(), "json");
        assert!(config.ignore.paths.contains(&"generated/".into()));
        assert!(config.ignore.globs.contains(&"*.cache".into()));

        Ok(())
    }

    #[test]
    fn env_overrides_take_precedence() -> Result<()> {
        let overrides = EnvOverrides::for_tests("openai:gpt-test", "plain");
        let config = Config::load_with_layers(None, None, overrides)?;
        assert_eq!(config.defaults.model(), "openai:gpt-test");
        assert_eq!(config.defaults.export_format(), "plain");
        Ok(())
    }

    #[test]
    fn invalid_config_returns_error() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let file = temp.path().join("broken.toml");
        fs::write(&file, "this is not toml")?;
        let result = Config::from_file(&file);
        assert!(result.is_err());
        Ok(())
    }
}
