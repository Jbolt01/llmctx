//! Token estimation services.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, UNIX_EPOCH};

use anyhow::{Context, Result};
use tiktoken_rs::{CoreBPE, cl100k_base, o200k_base};

use crate::domain::model::{ContextBundle, SelectionItem};
use crate::infra::config::Config;

/// Supported token estimation models across providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TokenModel {
    /// OpenAI GPT-4o (128k context window).
    OpenAiGpt4o,
    /// OpenAI GPT-4o mini (128k context window with faster pricing).
    #[default]
    OpenAiGpt4oMini,
    /// Anthropic Claude 3 Haiku (200k context window).
    AnthropicClaude3Haiku,
    /// Anthropic Claude 3.5 Sonnet (200k context window).
    AnthropicClaude35Sonnet,
    /// Generic character/word heuristic fallback.
    CharacterFallback,
}

impl TokenModel {
    /// Return a stable identifier suitable for serialization.
    pub fn as_str(&self) -> &'static str {
        match self {
            TokenModel::OpenAiGpt4o => "openai:gpt-4o",
            TokenModel::OpenAiGpt4oMini => "openai:gpt-4o-mini",
            TokenModel::AnthropicClaude3Haiku => "anthropic:claude-3-haiku",
            TokenModel::AnthropicClaude35Sonnet => "anthropic:claude-3.5-sonnet",
            TokenModel::CharacterFallback => "fallback:characters",
        }
    }

    /// Provider label for display purposes.
    pub fn provider(&self) -> &'static str {
        match self {
            TokenModel::OpenAiGpt4o | TokenModel::OpenAiGpt4oMini => "OpenAI",
            TokenModel::AnthropicClaude3Haiku | TokenModel::AnthropicClaude35Sonnet => "Anthropic",
            TokenModel::CharacterFallback => "Heuristic",
        }
    }

    /// Maximum context window for the model.
    pub fn context_window(&self) -> usize {
        match self {
            TokenModel::OpenAiGpt4o | TokenModel::OpenAiGpt4oMini => 128_000,
            TokenModel::AnthropicClaude3Haiku | TokenModel::AnthropicClaude35Sonnet => 200_000,
            TokenModel::CharacterFallback => 120_000,
        }
    }

    /// Enumerate all known models in priority order.
    pub fn all() -> &'static [TokenModel] {
        &[
            TokenModel::OpenAiGpt4o,
            TokenModel::OpenAiGpt4oMini,
            TokenModel::AnthropicClaude3Haiku,
            TokenModel::AnthropicClaude35Sonnet,
            TokenModel::CharacterFallback,
        ]
    }
}

impl fmt::Display for TokenModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for TokenModel {
    type Err = TokenModelParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "openai:gpt-4o" => Ok(TokenModel::OpenAiGpt4o),
            "openai:gpt-4o-mini" => Ok(TokenModel::OpenAiGpt4oMini),
            "anthropic:claude-3-haiku" => Ok(TokenModel::AnthropicClaude3Haiku),
            "anthropic:claude-3.5-sonnet" => Ok(TokenModel::AnthropicClaude35Sonnet),
            "fallback:characters" | "heuristic" | "fallback" => Ok(TokenModel::CharacterFallback),
            other => Err(TokenModelParseError::UnknownModel(other.to_string())),
        }
    }
}

/// Error returned when parsing a [`TokenModel`] fails.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum TokenModelParseError {
    #[error("unknown token model '{0}'")]
    UnknownModel(String),
}

/// Configurable heuristics used whenever a deterministic tokenizer is unavailable.
#[derive(Debug, Clone)]
pub struct HeuristicConfig {
    /// Average number of characters per token for non-code text.
    pub default_chars_per_token: f32,
    /// Average number of characters per token for Anthropic models.
    pub anthropic_chars_per_token: f32,
    /// Tokens per whitespace separated word (guards against very short words).
    pub tokens_per_word: f32,
    /// Multiplier applied when a selection is likely source code.
    pub code_token_multiplier: f32,
}

impl Default for HeuristicConfig {
    fn default() -> Self {
        Self {
            default_chars_per_token: 4.0,
            anthropic_chars_per_token: 3.2,
            tokens_per_word: 1.0,
            code_token_multiplier: 1.25,
        }
    }
}

impl HeuristicConfig {
    fn chars_per_token_for(&self, model: TokenModel) -> f32 {
        match model {
            TokenModel::AnthropicClaude3Haiku | TokenModel::AnthropicClaude35Sonnet => {
                self.anthropic_chars_per_token
            }
            _ => self.default_chars_per_token,
        }
    }

    fn estimate(&self, text: &str, model: TokenModel, is_code: bool) -> usize {
        if text.trim().is_empty() {
            return 0;
        }
        let chars = text.chars().count() as f32;
        let words = count_words(text) as f32;
        let char_based = (chars / self.chars_per_token_for(model)).ceil();
        let word_based = (words * self.tokens_per_word).ceil();
        let mut estimate = char_based.max(word_based) as usize;
        if is_code {
            estimate = ((estimate as f32) * self.code_token_multiplier).ceil() as usize;
        }
        estimate.max(1)
    }
}

/// Token estimation engine with caching and streaming updates.
#[derive(Debug, Clone)]
pub struct TokenEstimator {
    model: TokenModel,
    token_budget: u32,
    heuristics: HeuristicConfig,
    cache: Arc<Mutex<HashMap<CacheKey, ItemTokenEstimate>>>,
}

impl Default for TokenEstimator {
    fn default() -> Self {
        Self::new(TokenModel::default())
    }
}

impl TokenEstimator {
    /// Create a new estimator for the provided model using default heuristics.
    pub fn new(model: TokenModel) -> Self {
        Self {
            model,
            token_budget: 120_000,
            heuristics: HeuristicConfig::default(),
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Initialize from the layered application configuration.
    pub fn from_config(config: &Config) -> Self {
        let model = config
            .defaults
            .model()
            .parse()
            .unwrap_or_else(|_| TokenModel::default());
        let mut estimator = Self::new(model);
        estimator.token_budget = config.defaults.token_budget();
        estimator
    }

    /// Override the active model.
    pub fn set_model(&mut self, model: TokenModel) {
        if self.model != model {
            self.cache.lock().unwrap().clear();
            self.model = model;
        }
    }

    /// Returns the currently configured model.
    pub fn model(&self) -> TokenModel {
        self.model
    }

    /// Returns the configured token budget.
    pub fn token_budget(&self) -> u32 {
        self.token_budget
    }

    /// Update the configured token budget.
    pub fn set_token_budget(&mut self, budget: u32) {
        self.token_budget = budget;
    }

    /// Replace the heuristic configuration.
    pub fn set_heuristics(&mut self, heuristics: HeuristicConfig) {
        self.heuristics = heuristics;
        self.cache.lock().unwrap().clear();
    }

    /// Estimate tokens for the provided bundle, returning per-item breakdowns.
    pub fn estimate_bundle(&self, bundle: &ContextBundle) -> Result<BundleTokenSummary> {
        let model = bundle
            .model
            .as_deref()
            .and_then(|value| TokenModel::from_str(value).ok())
            .unwrap_or(self.model);

        let mut items = Vec::with_capacity(bundle.items.len());
        let mut total_tokens = 0usize;
        let mut total_characters = 0usize;

        for item in &bundle.items {
            let estimate = self.estimate_item(model, item)?;
            total_tokens += estimate.tokens;
            total_characters += estimate.characters;
            items.push(estimate);
        }

        Ok(BundleTokenSummary {
            model,
            token_budget: self.token_budget,
            total_tokens,
            total_characters,
            items,
        })
    }

    /// Invalidate cached entries for the given path.
    pub fn invalidate_path(&self, path: &Path) {
        let mut cache = self.cache.lock().unwrap();
        cache.retain(|key, _| key.path != path);
    }

    fn estimate_item(&self, model: TokenModel, item: &SelectionItem) -> Result<ItemTokenEstimate> {
        let fingerprint = file_fingerprint(&item.path);
        let key = CacheKey {
            model,
            path: item.path.clone(),
            range: item.range,
            fingerprint,
        };

        if let Some(existing) = self.cache.lock().unwrap().get(&key).cloned() {
            return Ok(existing);
        }

        let contents = load_selection_contents(item)
            .with_context(|| format!("failed to read selection '{}'", item.path.display()))?;
        let characters = contents.chars().count();
        let tokens = self.count_tokens(model, item, &contents);

        let estimate = ItemTokenEstimate {
            item: item.clone(),
            tokens,
            characters,
        };

        self.cache.lock().unwrap().insert(key, estimate.clone());

        Ok(estimate)
    }

    fn count_tokens(&self, model: TokenModel, item: &SelectionItem, contents: &str) -> usize {
        if contents.trim().is_empty() {
            return 0;
        }

        match tokenizer_for(model) {
            Ok(Tokenizer::Bpe(core)) => core.lock().unwrap().encode_ordinary(contents).len(),
            Ok(Tokenizer::Heuristic) | Err(_) => {
                self.heuristics
                    .estimate(contents, model, is_probably_code(&item.path))
            }
        }
    }
}

/// Summary of token counts for a [`ContextBundle`].
#[derive(Debug, Clone)]
pub struct BundleTokenSummary {
    pub model: TokenModel,
    pub token_budget: u32,
    pub total_tokens: usize,
    pub total_characters: usize,
    pub items: Vec<ItemTokenEstimate>,
}

/// Per-selection token estimate.
#[derive(Debug, Clone)]
pub struct ItemTokenEstimate {
    pub item: SelectionItem,
    pub tokens: usize,
    pub characters: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CacheKey {
    model: TokenModel,
    path: PathBuf,
    range: Option<(usize, usize)>,
    fingerprint: Option<FileFingerprint>,
}

impl Hash for CacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.model.hash(state);
        self.path.hash(state);
        self.range.hash(state);
        self.fingerprint.hash(state);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FileFingerprint {
    len: u64,
    modified: Option<u128>,
}

fn file_fingerprint(path: &Path) -> Option<FileFingerprint> {
    let metadata = fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(duration_to_nanos);

    Some(FileFingerprint {
        len: metadata.len(),
        modified,
    })
}

fn duration_to_nanos(duration: Duration) -> u128 {
    duration.as_secs() as u128 * 1_000_000_000u128 + duration.subsec_nanos() as u128
}

fn load_selection_contents(item: &SelectionItem) -> Result<String> {
    let raw = fs::read(&item.path)
        .with_context(|| format!("failed to read file '{}'", item.path.display()))?;
    let mut text = String::from_utf8_lossy(&raw).into_owned();
    if let Some((start, end)) = item.range {
        let start_idx = start.saturating_sub(1);
        let end_idx = end.max(start_idx);
        let lines: Vec<&str> = text.lines().collect();
        if start_idx >= lines.len() {
            text.clear();
        } else {
            let end_idx = end_idx.min(lines.len());
            text = lines[start_idx..end_idx].join("\n");
        }
    }
    Ok(text)
}

fn count_words(text: &str) -> usize {
    text.split_whitespace()
        .filter(|segment| !segment.is_empty())
        .count()
}

fn is_probably_code(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext,
                "rs" | "ts"
                    | "js"
                    | "jsx"
                    | "tsx"
                    | "py"
                    | "java"
                    | "c"
                    | "cpp"
                    | "cc"
                    | "h"
                    | "hpp"
                    | "go"
                    | "rb"
                    | "php"
                    | "cs"
                    | "swift"
                    | "scala"
                    | "kt"
                    | "sh"
                    | "zsh"
                    | "fish"
            )
        })
        .unwrap_or(false)
}

enum Tokenizer {
    Bpe(Arc<Mutex<CoreBPE>>),
    Heuristic,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
enum TokenizerInitError {
    #[error("failed to initialize OpenAI tokenizer: {0}")]
    OpenAi(String),
    #[error("failed to initialize Anthropic tokenizer: {0}")]
    Anthropic(String),
}

fn tokenizer_for(model: TokenModel) -> Result<Tokenizer, TokenizerInitError> {
    match model {
        TokenModel::OpenAiGpt4o | TokenModel::OpenAiGpt4oMini => {
            gpt4o_tokenizer().map(Tokenizer::Bpe)
        }
        TokenModel::AnthropicClaude3Haiku | TokenModel::AnthropicClaude35Sonnet => {
            claude_tokenizer().map(Tokenizer::Bpe)
        }
        TokenModel::CharacterFallback => Ok(Tokenizer::Heuristic),
    }
}

fn gpt4o_tokenizer() -> Result<Arc<Mutex<CoreBPE>>, TokenizerInitError> {
    static GPT4O: OnceLock<Result<Arc<Mutex<CoreBPE>>, TokenizerInitError>> = OnceLock::new();
    GPT4O
        .get_or_init(|| {
            o200k_base()
                .map(|bpe| Arc::new(Mutex::new(bpe)))
                .map_err(|err| TokenizerInitError::OpenAi(err.to_string()))
        })
        .clone()
}

fn claude_tokenizer() -> Result<Arc<Mutex<CoreBPE>>, TokenizerInitError> {
    static CLAUDE: OnceLock<Result<Arc<Mutex<CoreBPE>>, TokenizerInitError>> = OnceLock::new();
    CLAUDE
        .get_or_init(|| {
            cl100k_base()
                .map(|bpe| Arc::new(Mutex::new(bpe)))
                .map_err(|err| TokenizerInitError::Anthropic(err.to_string()))
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Write;

    use tempfile::NamedTempFile;

    fn temp_selection(contents: &str) -> (SelectionItem, NamedTempFile) {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        let item = SelectionItem {
            path: file.path().to_path_buf(),
            range: None,
            note: None,
        };
        (item, file)
    }

    #[test]
    fn parses_token_models_from_strings() {
        assert_eq!(
            TokenModel::from_str("openai:gpt-4o").unwrap(),
            TokenModel::OpenAiGpt4o
        );
        assert_eq!(
            TokenModel::from_str("OPENAI:GPT-4O-MINI").unwrap(),
            TokenModel::OpenAiGpt4oMini
        );
        assert_eq!(
            TokenModel::from_str("anthropic:claude-3-haiku").unwrap(),
            TokenModel::AnthropicClaude3Haiku
        );
        assert!(TokenModel::from_str("unknown").is_err());
    }

    #[test]
    fn estimates_tokens_with_openai_tokenizer() {
        let (selection, _temp) = temp_selection("Hello world!");
        let bundle = ContextBundle {
            items: vec![selection.clone()],
            model: Some("openai:gpt-4o".into()),
        };
        let estimator = TokenEstimator::new(TokenModel::OpenAiGpt4o);
        let summary = estimator.estimate_bundle(&bundle).unwrap();
        assert_eq!(summary.total_tokens, 3);
        assert_eq!(summary.items[0].tokens, 3);
        assert_eq!(summary.total_characters, "Hello world!".chars().count());
    }

    #[test]
    fn estimates_tokens_with_anthropic_tokenizer() {
        let (selection, _temp) = temp_selection("Claude likes accurate token counts.");
        let bundle = ContextBundle {
            items: vec![selection.clone()],
            model: Some("anthropic:claude-3.5-sonnet".into()),
        };
        let estimator = TokenEstimator::new(TokenModel::AnthropicClaude35Sonnet);
        let summary = estimator.estimate_bundle(&bundle).unwrap();
        assert_eq!(summary.items.len(), 1);
        assert!(summary.total_tokens > 0);
        assert_eq!(summary.total_tokens, summary.items[0].tokens);
    }

    #[test]
    fn applies_range_selection_when_present() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "fn main() {{}}").unwrap();
        writeln!(file, "// comment").unwrap();
        writeln!(file, "println!(\"done\");").unwrap();
        let selection = SelectionItem {
            path: file.path().to_path_buf(),
            range: Some((2, 3)),
            note: None,
        };
        let bundle = ContextBundle {
            items: vec![selection],
            model: Some("openai:gpt-4o-mini".into()),
        };
        let estimator = TokenEstimator::new(TokenModel::OpenAiGpt4oMini);
        let summary = estimator.estimate_bundle(&bundle).unwrap();
        assert!(summary.total_tokens > 0);
        assert!(summary.total_characters < "fn main() {}\n// comment\nprintln!(\"done\");\n".len());
    }

    #[test]
    fn falls_back_to_heuristics() {
        let (selection, _temp) = temp_selection("Approximate counting is good enough.");
        let bundle = ContextBundle {
            items: vec![selection.clone()],
            model: Some("fallback:characters".into()),
        };
        let estimator = TokenEstimator::new(TokenModel::CharacterFallback);
        let summary = estimator.estimate_bundle(&bundle).unwrap();
        assert!(summary.total_tokens > 0);
    }

    #[test]
    fn cache_invalidation_follows_file_changes() {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "Hello world!").unwrap();
        let selection = SelectionItem {
            path: file.path().to_path_buf(),
            range: None,
            note: None,
        };
        let bundle = ContextBundle {
            items: vec![selection.clone()],
            model: Some("openai:gpt-4o".into()),
        };
        let estimator = TokenEstimator::new(TokenModel::OpenAiGpt4o);

        let first = estimator.estimate_bundle(&bundle).unwrap();
        assert_eq!(first.total_tokens, 3);

        estimator.invalidate_path(&selection.path);
        write!(file.as_file_mut(), " More text").unwrap();
        file.flush().unwrap();

        let second = estimator.estimate_bundle(&bundle).unwrap();
        assert!(second.total_tokens >= first.total_tokens);
    }

    #[test]
    fn estimator_respects_config_defaults() {
        let config: Config = toml::from_str(
            r#"
            [defaults]
            model = "anthropic:claude-3-haiku"
            token_budget = 42000
            "#,
        )
        .unwrap();
        let estimator = TokenEstimator::from_config(&config);
        assert_eq!(estimator.model(), TokenModel::AnthropicClaude3Haiku);
        assert_eq!(estimator.token_budget(), 42_000);
    }
}
