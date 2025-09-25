# Testing Strategy

This guide explains how llmctx structures automated tests locally and in CI. It also captures the roadmap for coverage tooling so contributors understand the expectations as the suite grows.

## Test Layout

- **Unit tests** live alongside modules using the standard `#[cfg(test)]` convention.
- **Integration tests** belong in `tests/`. These scenarios can shell out to the binary, exercise the CLI, or mock infrastructure components.
- **Fixture state** should be created dynamically within each test via helpers. Avoid static files in the repository unless a test specifically covers parsing existing content.

## Local Execution

Run the same commands exercised in CI to ensure green builds:

```sh
# Fast feedback: debug profile
cargo test --all-features --no-fail-fast

# Release profile parity with CI (optimisation differences, feature gating)
cargo test --all-features --release --no-fail-fast
```

The `--no-fail-fast` flag ensures CI produces the complete failure set; keep it locally when investigating cascades.

### Generating JUnit Reports Locally

CI publishes JUnit XML artifacts for downstream tooling. To reproduce locally:

```sh
# Install once (skipped if already present)
cargo install cargo2junit --locked

cargo test --all-features -- --format json --report-time \
  | cargo2junit > target/dev-junit.xml

cargo test --all-features --release -- --format json --report-time \
  | cargo2junit > target/release-junit.xml
```

JUnit files live under `target/` and can be ingested by IDEs or reporting dashboards.

## Integration Test Helpers

The CI workflow creates `target/test-fixtures/default-config.json` before running tests. Use `crate::app::test_support` (or create a helper module) to read from `target/test-fixtures/` so tests can rely on predictable scratch space without polluting the repository.

## Coverage Roadmap

We plan to introduce `cargo tarpaulin` once the test suite stabilises (tracked in Issue #17 follow-ups). The governance doc captures how the coverage stage will integrate with CI, including:

- Running coverage in nightly builds or on-demand via workflow dispatch to keep PRs fast.
- Publishing coverage summaries as GitHub check annotations and badges.
- Optionally failing below-threshold coverage once baselines are trusted.

Until then, contributors should keep tests focused, deterministic, and well-documented.

## Failure Investigation Tips

1. Download the `junit-<profile>` and `test-logs-<profile>` artifacts from CI.
2. Inspect JSON logs for the failing test name and output.
3. Re-run the failing test with `cargo test <test_path> -- --nocapture`.
4. Use `RUST_BACKTRACE=1` for additional context.

Keeping tests reliable ensures llmctx remains stable as the feature set grows.
