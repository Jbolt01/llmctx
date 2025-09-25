# CI Governance Strategy

This document defines how llmctx ensures every change is vetted through automated validation before merge. It captures the agreed set of pipeline stages, required signals, and expectations for maintainers and contributors.

## Objectives

- Prevent regressions in formatting, lint hygiene, functionality, and security.
- Provide deterministic, actionable feedback inside GitHub pull requests.
- Keep the pipeline modular so stages can evolve independently.
- Document branch protection and on-call conventions for CI health.

## Pipeline Overview

The workflow name is `CI`. It contains the following stages, executed as separate jobs:

| Job | Purpose | Trigger | Required Status |
| --- | --- | --- | --- |
| `governance-lint` | Formatting and Clippy linting | PRs, pushes to `main` | Required |
| `governance-test` | Unit & integration tests (dev + release) | PRs, pushes to `main` | Required |
| `governance-build` | Verifies release build compiles | PRs, pushes to `main` | Required |
| `governance-security` | Security placeholder (cargo audit, SBOM) | PRs, pushes to `main` | Required once tooling lands |

The workflow is defined in `.github/workflows/ci.yml` and leverages a reusable setup that installs the stable Rust toolchain, caches dependencies, and emits job summaries for quick triage.

### Job Details

**governance-lint**
- Runs `cargo fmt --all -- --check` to enforce formatting.
- Runs `cargo clippy --all-targets --all-features -- -D warnings` to block regressions.
- Fails fast so contributors address style issues before functional failures.

**governance-test**
- Executes a matrix across debug (`cargo test --all-features --no-fail-fast`) and release (`cargo test --all-features --release --no-fail-fast`) profiles.
- Captures JSON output and converts it with `cargo2junit`, uploading artifacts for dashboards or IDE consumption.
- Archives raw logs when failures occur and prepares scratch fixtures under `target/test-fixtures/` for integration tests.
- Future scope: add optional coverage jobs (`cargo tarpaulin`) and OS matrices once runtime analysis justifies it.

**governance-build**
- Builds optimized binaries via `cargo build --workspace --release`.
- Ensures release artifacts compile before we invest in packaging workflows.
- Future scope: expand to matrix across `ubuntu-latest` and `macos-latest`; document Windows/ARM backlog.

**governance-security**
- Placeholder job that currently records the expectation for automated security scanning.
- To be expanded by Issue #18 (`DevOps: Security scanning & dependency hygiene`).
- Will eventually run `cargo audit`, generate SBOMs, and fail on high severity vulnerabilities.

## Branch Protection

Branch protection for `main` must require the following:

- PR must be up to date with base (`Require branches to be up to date before merging`).
- Status checks: `governance-lint`, `governance-test`, `governance-build` (and `governance-security` once implemented).
- `Require pull request reviews before merging` with at least one approval.
- `Dismiss stale pull request approvals when new commits are pushed` to avoid bypassing checks.

These settings are documented in place of immediate configuration changes to avoid automation conflicts; maintainers should update repository settings accordingly.

## Notification & Triage

- Each job posts a GitHub Actions job summary with the key commands that ran, execution time, and next steps for failures.
- Failed jobs block merge and appear under the PR status section. Maintainers mention `@codex review` when requesting a re-review after addressing failures.
- CI failure rotation: the on-call reviewer for the week acknowledges failing checks within 24 hours, tags the contributor if action is required, and opens follow-up issues for systemic problems (e.g., flaky tests).

## Platform Strategy

- Default runner: `ubuntu-latest` for all jobs today.
- Matrix expansion: document needs for `macos-latest` when preview rendering or clipboard functionality requires OS-specific coverage. Windows/ARM tracked in a future issue if distribution demands it.
- The `governance-build` job is the first candidate for multi-OS coverage once caching and runtime investigations are complete.

## Evolution Plan

- Issue #16 will split linting and formatting into more granular checks if runtime exceeds 3 minutes per job.
- Issue #17 introduces richer integration test harnesses and optional coverage tooling.
- Issue #18 replaces the security placeholder with real scanners.
- Issue #19 hooks release artifact publishing onto successful builds.
- Issue #20 captures telemetry from these jobs to guide further optimizations.

## Contributor Checklist

Before pushing:

1. Run `cargo fmt --all`.
2. Run `cargo clippy --all-targets --all-features -- -D warnings`.
3. Run `cargo test --all-features`.
4. (Optional) Run `cargo build --workspace --release` to preempt build job failures.

Keep PRs targeted at a single scope, use conventional commits, and link the relevant issue (e.g., `Closes #15`).
