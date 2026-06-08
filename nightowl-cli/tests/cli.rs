//! End-to-end tests for the `nightowl-cli` binary.
//!
//! Each test invokes the compiled binary against its own temp data
//! directory, so the live data on the machine running the tests is
//! never touched and the tests do not interfere with each other.

use std::process::Command;

use assert_cmd::prelude::*;
use tempfile::TempDir;

/// Spawns the binary under test against an isolated data directory.
/// Returns the temp dir so the caller can hold it open for the
/// lifetime of the assertion (drop = cleanup).
fn run_in_tempdir(args: &[&str]) -> (TempDir, std::process::Output) {
    let tmp = TempDir::new().expect("tempdir");
    let mut cmd = Command::cargo_bin("nightowl-cli").expect("locate binary");
    cmd.arg("--data-dir").arg(tmp.path());
    for a in args {
        cmd.arg(a);
    }
    let out = cmd.output().expect("spawn nightowl-cli");
    (tmp, out)
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8(out.stdout.clone()).expect("utf8 stdout")
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8(out.stderr.clone()).expect("utf8 stderr")
}

#[test]
fn config_show_human_lists_paths_and_defaults() {
    let (_tmp, out) = run_in_tempdir(&["config", "show"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let body = stdout(&out);
    assert!(body.contains("local_ae_title: NIGHTOWL"));
    assert!(body.contains("listen_port:    11112"));
    assert!(body.contains("mcp.enabled:    false"));
}

#[test]
fn config_show_json_is_valid_json_with_expected_fields() {
    let (_tmp, out) = run_in_tempdir(&["--json", "config", "show"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let v: serde_json::Value =
        serde_json::from_str(&stdout(&out)).expect("config show --json must parse");
    assert_eq!(v["local_ae_title"], "NIGHTOWL");
    assert_eq!(v["listen_port"], 11112);
    assert_eq!(v["mcp"]["enabled"], false);
}

#[test]
fn empty_dir_peers_list_reports_empty() {
    let (_tmp, out) = run_in_tempdir(&["peers", "list"]);
    assert!(out.status.success());
    assert_eq!(stdout(&out).trim(), "(no peers configured)");
}

#[test]
fn empty_dir_instances_count_is_zero() {
    let (_tmp, out) = run_in_tempdir(&["instances", "count"]);
    assert!(out.status.success());
    assert_eq!(stdout(&out).trim(), "0");
}

#[test]
fn empty_dir_activity_count_is_zero() {
    let (_tmp, out) = run_in_tempdir(&["activity", "count"]);
    assert!(out.status.success());
    assert_eq!(stdout(&out).trim(), "0");
}

#[test]
fn empty_dir_studies_list_reports_empty() {
    let (_tmp, out) = run_in_tempdir(&["studies", "list"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("(no studies in store)"));
}

#[test]
fn empty_dir_worklist_list_reports_empty() {
    let (_tmp, out) = run_in_tempdir(&["worklist", "list"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("(no worklist entries)"));
}

#[test]
fn unknown_peer_id_is_validation_failure_with_exit_2() {
    let (_tmp, out) = run_in_tempdir(&["scu", "echo", "nope"]);
    assert!(!out.status.success(), "should have exited non-zero");
    assert_eq!(out.status.code(), Some(2), "validation → exit code 2");
    assert!(
        stderr(&out).contains("unknown peer id nope"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn missing_subcommand_exits_with_help() {
    // No subcommand at all — clap should print help and exit non-zero.
    let mut cmd = Command::cargo_bin("nightowl-cli").expect("locate binary");
    let out = cmd.output().expect("spawn");
    assert!(!out.status.success());
    // Help text goes to stderr by default for clap errors.
    assert!(stderr(&out).contains("Usage:"));
}

#[test]
fn opens_existing_dir_twice_without_conflict() {
    // Two back-to-back invocations against the same data dir should
    // both succeed. This is the regression guard for WAL concurrency:
    // if SQLite or the JSON peer-store grew an exclusive lock, the
    // second call would fail.
    let tmp = TempDir::new().expect("tempdir");
    for _ in 0..2 {
        let out = Command::cargo_bin("nightowl-cli")
            .unwrap()
            .arg("--data-dir")
            .arg(tmp.path())
            .args(["instances", "count"])
            .output()
            .expect("spawn");
        assert!(out.status.success(), "stderr: {}", stderr(&out));
    }
}
