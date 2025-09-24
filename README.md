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

## CI
GitHub Actions workflow runs fmt, clippy, and tests on pushes and pull requests.
