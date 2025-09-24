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
    #[serde(default = "Defaults::default_model")]
    pub model: String,
    #[serde(default = "Defaults::default_export_format")]
    pub export_format: String,
    #[serde(default = "Defaults::default_token_budget")]
    pub token_budget: u32,
    #[serde(default = "Defaults::default_theme")]
    pub theme: String,
    #[serde(default = "Defaults::default_preview_max_lines")]
    pub preview_max_lines: usize,
    #[serde(default)]
    pub show_hidden: bool,
}

impl Defaults {
    fn default_model() -> String {
        "openai:gpt-4o-mini".to_owned()
    }

    fn default_export_format() -> String {
        "markdown".into()
    }

    fn default_token_budget() -> u32 {
        120_000
    }

    fn default_theme() -> String {
        "dracula".into()
    }

    fn default_preview_max_lines() -> usize {
        400
    }
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            model: Self::default_model(),
            export_format: Self::default_export_format(),
            token_budget: Self::default_token_budget(),
            theme: Self::default_theme(),
            preview_max_lines: Self::default_preview_max_lines(),
            show_hidden: false,
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

fn merge_defaults(base: Defaults, overlay: Defaults) -> Defaults {
    Defaults {
        model: if overlay.model != Defaults::default_model() {
            overlay.model
        } else {
            base.model
        },
        export_format: if overlay.export_format != Defaults::default_export_format() {
            overlay.export_format
        } else {
            base.export_format
        },
        token_budget: if overlay.token_budget != Defaults::default_token_budget() {
            overlay.token_budget
        } else {
            base.token_budget
        },
        theme: if overlay.theme != Defaults::default_theme() {
            overlay.theme
        } else {
            base.theme
        },
        preview_max_lines: if overlay.preview_max_lines != Defaults::default_preview_max_lines() {
            overlay.preview_max_lines
        } else {
            base.preview_max_lines
        },
        show_hidden: overlay.show_hidden || base.show_hidden,
    }
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
        config.defaults.model = model;
    }
    if let Some(export_format) = env.export_format {
        config.defaults.export_format = export_format;
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
        assert_eq!(config.defaults.model, "openai:gpt-4o-mini");
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

        assert_eq!(config.defaults.model, "anthropic:claude");
        assert_eq!(config.defaults.export_format, "json");
        assert!(config.ignore.paths.contains(&"generated/".into()));
        assert!(config.ignore.globs.contains(&"*.cache".into()));

        Ok(())
    }

    #[test]
    fn env_overrides_take_precedence() -> Result<()> {
        let overrides = EnvOverrides::for_tests("openai:gpt-test", "plain");
        let config = Config::load_with_layers(None, None, overrides)?;
        assert_eq!(config.defaults.model, "openai:gpt-test");
        assert_eq!(config.defaults.export_format, "plain");
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
