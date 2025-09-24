# llmctx

llmctx is a Rust-based terminal UI for assembling curated code context for large language models.

## Prerequisites
- Rust toolchain (stable). Install via [rustup](https://rustup.rs/).
- `cargo` in your PATH.

## Getting Started
```sh
cargo run -p llmctx
```

## Project Structure
- `Cargo.toml`: Workspace manifest.
- `crates/llmctx`: Main binary crate.
  - `src/app`: Application services (scan, search, selection, tokens, export, session).
  - `src/domain`: Domain models and errors.
  - `src/infra`: Infrastructure adapters (fs, git, config, plugins, logging).
  - `src/ui`: ratatui components and app loop.

## Development
- Format: `cargo fmt --all`
- Lint: `cargo clippy --all-targets --all-features`
- Test: `cargo test`

## Configuration
llmctx loads settings from the following layers (later entries override earlier ones):

1. Built-in defaults (bundled `crates/llmctx/assets/default-config.toml`).
2. User config at `${XDG_CONFIG_HOME:-~/.config}/llmctx/config.toml`.
3. Workspace override at `<repo>/.llmctx/config.toml`.
4. Environment variables (`LLMCTX_MODEL`, `LLMCTX_EXPORT_FORMAT`).

Example TOML:

```toml
[defaults]
model = "openai:gpt-4o-mini"
export_format = "markdown"
token_budget = 120000
theme = "dracula"
preview_max_lines = 400
show_hidden = false

[ignore]
paths = ["target/", "dist/"]
globs = ["*.lock"]

[export]
include_git_metadata = true
include_line_numbers = true
template = "concise_context"

[keybindings]
up = "k"
down = "j"
select = "space"
export = "ctrl+e"

[preview]
theme = "dracula"
max_lines = 400
load_more_step = 200
```

## CI
GitHub Actions workflow runs fmt, clippy, and tests on pushes and pull requests.
