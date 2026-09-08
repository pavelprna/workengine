use std::path::Path;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_workengine"))
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn create_work(data_dir: &Path) -> String {
    let output = bin()
        .args([
            "--data-dir",
            data_dir.to_str().unwrap(),
            "create",
            "--goal",
            "do the thing",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout(&output)
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned()
}

#[test]
fn version_prints_package_version() {
    let output = bin().arg("version").output().unwrap();
    assert!(output.status.success());
    assert_eq!(stdout(&output), "workengine 0.0.0");
}

#[test]
fn create_next_start_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let id = create_work(dir.path());
    let next = bin()
        .args(["--data-dir", dir.path().to_str().unwrap(), "next"])
        .output()
        .unwrap();
    assert!(next.status.success());
    assert_eq!(stdout(&next), id);
    let start = bin()
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "start",
            "--work",
            &id,
        ])
        .output()
        .unwrap();
    assert!(
        start.status.success(),
        "start failed: {}",
        String::from_utf8_lossy(&start.stderr)
    );
    assert!(stdout(&start).ends_with(" succeeded"));
    let next_again = bin()
        .args(["--data-dir", dir.path().to_str().unwrap(), "next"])
        .output()
        .unwrap();
    assert!(next_again.status.success());
    assert!(stdout(&next_again).is_empty());
}

#[test]
fn start_after_success_is_illegal_transition() {
    let dir = tempfile::tempdir().unwrap();
    let id = create_work(dir.path());
    let first = bin()
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "start",
            "--work",
            &id,
        ])
        .output()
        .unwrap();
    assert!(first.status.success());
    let second = bin()
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "start",
            "--work",
            &id,
        ])
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(10));
}

#[test]
fn complete_file_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let id = create_work(dir.path());
    let start = bin()
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "start",
            "--work",
            &id,
        ])
        .output()
        .unwrap();
    assert!(start.status.success());
    let outcome = dir.path().join("workspaces").join(&id).join("outcome.json");
    let again = bin()
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "complete",
            "--work",
            &id,
            "--file",
            outcome.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        again.status.success(),
        "complete failed: {}",
        String::from_utf8_lossy(&again.stderr)
    );
    assert!(stdout(&again).ends_with(" succeeded"));
}

#[test]
fn unknown_outcome_kind_is_schema_exit() {
    let dir = tempfile::tempdir().unwrap();
    let id = create_work(dir.path());
    let file = dir.path().join("bad.json");
    std::fs::write(
        &file,
        r#"{"schemaVersion":1,"kind":"needs_review","workerProfile":"stub"}"#,
    )
    .unwrap();
    let output = bin()
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "complete",
            "--work",
            &id,
            "--file",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(30));
}

#[test]
fn no_args_is_usage_error() {
    let output = bin().output().unwrap();
    assert_eq!(output.status.code(), Some(2));
}
