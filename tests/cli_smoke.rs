//! Smoke tests for the grel CLI binary.

use std::process::Command;

fn grel_bin() -> &'static str {
    env!("CARGO_BIN_EXE_grel")
}

#[test]
fn help_flag_prints_usage() {
    let output = Command::new(grel_bin())
        .arg("--help")
        .output()
        .expect("help command should run");

    assert!(output.status.success(), "help command should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage: grel <OPERATION> [OPTIONS] [TARGETS...]"));
    assert!(stdout.contains("A package manager for pre-built binaries from Git forges"));
}

#[test]
fn version_flag_prints_current_version() {
    let output = Command::new(grel_bin())
        .arg("--version")
        .output()
        .expect("version command should run");

    assert!(output.status.success(), "version command should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn invalid_forge_value_is_rejected() {
    let output = Command::new(grel_bin())
        .args(["--forge", "bogus"])
        .output()
        .expect("invalid forge command should run");

    assert!(!output.status.success(), "invalid forge should fail");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("possible values"));
}
