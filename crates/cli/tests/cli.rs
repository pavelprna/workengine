use std::path::Path;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_workengine"))
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn stream_records(stderr: &[u8]) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(stderr)
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn mark_leftover_running(data_dir: &Path, id: &str) {
    let conn = rusqlite::Connection::open(data_dir.join("workengine.sqlite")).unwrap();
    conn.execute("UPDATE works SET status = 'running' WHERE id = ?1", [id])
        .unwrap();
    let ws = data_dir.join("workspaces").join(id);
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(
        ws.join("outcome.json"),
        r#"{"schemaVersion":1,"kind":"succeeded","workerProfile":"stub"}"#,
    )
    .unwrap();
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
    let line = stdout(&output);
    assert!(line.starts_with("workengine 0.0.0"), "version line: {line}");
    if let Some(rest) = line.strip_prefix("workengine 0.0.0")
        && !rest.is_empty()
    {
        assert!(
            rest.starts_with(" (") && rest.ends_with(')'),
            "version line: {line}"
        );
        assert!(rest.len() > 3, "version line: {line}");
    }
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
fn start_after_success_completes_from_leftover_artifact() {
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
    assert!(
        second.status.success(),
        "second start should complete from leftover artifact: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(stdout(&second).ends_with(" succeeded"));
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
    let memory =
        std::fs::read_to_string(dir.path().join("workspaces").join(&id).join("memory.log"))
            .unwrap();
    assert_eq!(memory.lines().count(), 1);
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

#[test]
fn complete_file_after_next_recovers_parked_leftover() {
    let dir = tempfile::tempdir().unwrap();
    let id = create_work(dir.path());
    mark_leftover_running(dir.path(), &id);
    let next = bin()
        .args(["--data-dir", dir.path().to_str().unwrap(), "next"])
        .output()
        .unwrap();
    assert!(next.status.success());
    assert_eq!(stdout(&next), id);
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
        "complete after recover failed: {}",
        String::from_utf8_lossy(&again.stderr)
    );
    assert!(stdout(&again).ends_with(" succeeded"));
}

#[cfg(unix)]
#[test]
fn hang_exits_timed_out() {
    let dir = tempfile::tempdir().unwrap();
    let id = create_work(dir.path());
    let start = bin()
        .env("WORKENGINE_STUB_BEHAVIOR", "hang")
        .env("WORKENGINE_BUDGET_MS", "80")
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "start",
            "--work",
            &id,
        ])
        .output()
        .unwrap();
    assert_eq!(
        start.status.code(),
        Some(21),
        "hang stderr: {}",
        String::from_utf8_lossy(&start.stderr)
    );
    assert!(stdout(&start).ends_with(" failed"));
}

#[cfg(unix)]
#[test]
fn exceed_budget_exits_budget_exceeded() {
    let dir = tempfile::tempdir().unwrap();
    let id = create_work(dir.path());
    let start = bin()
        .env("WORKENGINE_STUB_BEHAVIOR", "exceed_budget")
        .env("WORKENGINE_BUDGET_MS", "80")
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "start",
            "--work",
            &id,
        ])
        .output()
        .unwrap();
    assert_eq!(
        start.status.code(),
        Some(20),
        "budget stderr: {}",
        String::from_utf8_lossy(&start.stderr)
    );
    assert!(stdout(&start).ends_with(" failed"));
}

#[test]
fn start_emits_stream_records_with_work_id() {
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
    assert!(
        start.status.success(),
        "start failed: {}",
        String::from_utf8_lossy(&start.stderr)
    );
    assert!(stdout(&start).ends_with(" succeeded"));
    let records = stream_records(&start.stderr);
    assert!(
        !records.is_empty(),
        "expected JSON stream on stderr, got: {}",
        String::from_utf8_lossy(&start.stderr)
    );
    for rec in &records {
        assert_eq!(rec["schemaVersion"], 1, "{rec}");
        let work_id = rec["workId"]
            .as_str()
            .unwrap_or_else(|| panic!("missing workId: {rec}"));
        assert_eq!(work_id, id, "{rec}");
        assert!(
            rec["event"].as_str().is_some_and(|e| !e.is_empty()),
            "{rec}"
        );
    }
    let events: Vec<&str> = records.iter().filter_map(|r| r["event"].as_str()).collect();
    assert!(events.contains(&"spawned"), "{events:?}");
    assert!(events.contains(&"exited"), "{events:?}");
    assert!(events.contains(&"child_stdout"), "{events:?}");
}

#[cfg(unix)]
#[test]
fn data_dir_lock_rejects_a_second_cli() {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let dir = tempfile::tempdir().unwrap();
    let id = create_work(dir.path());
    let mut child = bin()
        .env("WORKENGINE_STUB_BEHAVIOR", "hang")
        .env("WORKENGINE_BUDGET_MS", "4000")
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "start",
            "--work",
            &id,
        ])
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().expect("piped stderr");
    let mut lines = BufReader::new(stderr).lines();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut saw_spawned = false;
    while Instant::now() < deadline {
        let Some(Ok(line)) = lines.next() else {
            break;
        };
        if line.contains("\"event\":\"spawned\"") {
            saw_spawned = true;
            break;
        }
    }
    assert!(
        saw_spawned,
        "start never emitted spawned; it must hold the lock first"
    );
    let output = bin()
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "create",
            "--goal",
            "other",
        ])
        .output()
        .unwrap();
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(
        output.status.code(),
        Some(11),
        "second CLI stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn channel_park_exits_parked() {
    let dir = tempfile::tempdir().unwrap();
    let id = create_work(dir.path());
    let start = bin()
        .env("WORKENGINE_STUB_BEHAVIOR", "channel_park")
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
        "channel park stderr: {}",
        String::from_utf8_lossy(&start.stderr)
    );
    assert!(stdout(&start).ends_with(" parked"));
}

#[cfg(unix)]
#[test]
fn process_profile_writes_goal_and_uses_outcome_file() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("workengine.toml");
    std::fs::write(
        &config,
        r#"
[profile.writer]
argv = ["/bin/sh", "-c", "printf '%s\\n' '{\"schemaVersion\":1,\"kind\":\"succeeded\",\"workerProfile\":\"writer\"}' > outcome.json"]
"#,
    )
    .unwrap();
    let create = bin()
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "create",
            "--goal",
            "from-cli",
            "--profile",
            "writer",
        ])
        .output()
        .unwrap();
    assert!(
        create.status.success(),
        "create failed: {}",
        String::from_utf8_lossy(&create.stderr)
    );
    let id = stdout(&create)
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let start = bin()
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
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
    let goal = std::fs::read_to_string(
        dir.path()
            .join("workspaces")
            .join(&id)
            .join("workengine-goal.txt"),
    )
    .unwrap();
    assert_eq!(goal, "from-cli");
}

#[cfg(unix)]
#[test]
fn process_without_outcome_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("workengine.toml");
    std::fs::write(
        &config,
        r#"
[profile.true]
argv = ["/bin/true"]
"#,
    )
    .unwrap();
    let create = bin()
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "create",
            "--goal",
            "no-artifact",
            "--profile",
            "true",
        ])
        .output()
        .unwrap();
    assert!(create.status.success());
    let id = stdout(&create)
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let start = bin()
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
            "start",
            "--work",
            &id,
        ])
        .output()
        .unwrap();
    assert_eq!(
        start.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&start.stderr)
    );
    assert!(stdout(&start).ends_with(" failed"));
}

#[cfg(unix)]
#[test]
fn checkout_copy_and_lazy_profile_validation() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("hello.txt"), "copied").unwrap();
    let config = dir.path().join("workengine.toml");
    std::fs::write(
        &config,
        r#"
[profile.broken]
argv = ["/bin/false"]

[profile.broken.env]
TOKEN = "inline-secret"

[profile.writer]
argv = ["/bin/sh", "-c", "printf '%s\\n' '{\"schemaVersion\":1,\"kind\":\"succeeded\",\"workerProfile\":\"writer\"}' > outcome.json"]
"#,
    )
    .unwrap();
    let create = bin()
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "create",
            "--goal",
            "copy-me",
            "--profile",
            "writer",
        ])
        .output()
        .unwrap();
    assert!(create.status.success());
    let id = stdout(&create)
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let start = bin()
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
            "start",
            "--work",
            &id,
            "--checkout",
            src.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        start.status.success(),
        "start failed: {}",
        String::from_utf8_lossy(&start.stderr)
    );
    let copied =
        std::fs::read_to_string(dir.path().join("workspaces").join(&id).join("hello.txt")).unwrap();
    assert_eq!(copied, "copied");
}
