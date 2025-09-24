# llmctx

llmctx is a Rust-based terminal UI for assembling curated code context for large language models.

## Prerequisites
- Rust toolchain (stable). Install via [rustup](https://rustup.rs/).
- `cargo` in your PATH.

## Getting Started
```sh
cargo run -p llmctx
```

### Interactive TUI

Launching `cargo run -p llmctx` opens a full-screen terminal experience composed of:

- **Workspace tree** (left) – browse the repository, expand/collapse folders, and toggle selections.
- **Preview** (center) – syntax-highlighted file view with incremental loading for large files.
- **Selection summary** (right) – live token estimates and export readiness.
- **Command hints & status** (bottom) – discoverable shortcuts and contextual feedback.

#### Core keybindings

| Keys | Action |
| --- | --- |
| `j` / `↓` &nbsp;&nbsp;`k` / `↑` | Move through the file tree |
| `h` / `←` | Collapse directory or jump to parent |
| `l` / `→` / `Enter` | Expand directory or open preview |
| `Tab` | Switch between tree and preview panes |
| `Space` | Toggle whole-file selection |
| `Shift` + `↑` / `↓` | Grow or shrink a line range selection in the preview |
| `/` | Start incremental filter on the file tree |
| `:` | Open the command palette |
| `Ctrl+S` | Persist the current session to `.llmctx/session.json` |
| `Ctrl+E` | Export the active selection bundle (writes to `.llmctx/exports/` and copies to clipboard) |
| `q` / `Ctrl+Q` | Quit |

The command palette supports quick actions such as:

- `filter <pattern>` – apply a name filter to the file tree
- `select <start-end>` – add a specific line range for the active preview
- `export [path]` – write the current bundle to an explicit path
- `save` – persist selections and UI state
- `model <id>` – switch the active token model

Session state (tree filter, focused file, selections, and model override) is automatically reloaded on startup when `.llmctx/session.json` is present.

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

### Token estimation

The token estimator supports the following model identifiers:

- `openai:gpt-4o`
- `openai:gpt-4o-mini`
- `anthropic:claude-3-haiku`
- `anthropic:claude-3.5-sonnet`
- `fallback:characters` (heuristic character/word counter)

Set `defaults.model` in the configuration or `LLMCTX_MODEL` in the environment to switch the active model. `defaults.token_budget` defines the maximum context window displayed in the TUI summary. When a precise tokenizer is unavailable, llmctx falls back to configurable character/word heuristics so estimates remain available offline.

## Exporting Context

Selections can be exported directly from the command line without launching the TUI. Use the `export` subcommand to specify files or ranges and control output:

```sh
# Export two files using the default Markdown template, writing to stdout and clipboard
llmctx export src/lib.rs src/main.rs --copy --print

# Export a line range with an inline note using the plain text template
llmctx export \
  --select "src/app/mod.rs:10-80#core wiring" \
  --format plain \
  --template plain_text \
  --output context.txt
```

Selections accept the format `path[:start-end][#note]`. Ranges are inclusive and line-numbered output is enabled by default (configurable via `export.include_line_numbers`). The exporter respects configuration defaults for the target model, templates, and git metadata. Rendered output can be written to disk, copied to the clipboard, and/or printed to stdout in a single invocation.

## CI
GitHub Actions workflow runs fmt, clippy, and tests on pushes and pull requests.
