use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_displays_usage() {
    Command::cargo_bin("llmctx")
        .expect("binary exists")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage"));
}
