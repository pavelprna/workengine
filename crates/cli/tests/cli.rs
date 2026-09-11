use std::process::Command;
use std::time::{Duration, Instant};

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

fn create_work(data_dir: &std::path::Path) -> String {
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

fn force_running(data_dir: &std::path::Path, id: &str) {
    let database = data_dir.join("workengine.sqlite");
    let connection = rusqlite::Connection::open(database).unwrap();
    connection
        .execute("UPDATE works SET status = 'running' WHERE id = ?1", [id])
        .unwrap();
}

fn status_and_event_count(data_dir: &std::path::Path, id: &str) -> (String, i64) {
    let database = data_dir.join("workengine.sqlite");
    let connection = rusqlite::Connection::open(database).unwrap();
    let status = connection
        .query_row("SELECT status FROM works WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .unwrap();
    let events = connection
        .query_row(
            "SELECT COUNT(*) FROM events WHERE work_id = ?1",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    (status, events)
}

fn http_request(port: u16, request: &[u8]) -> std::io::Result<String> {
    use std::io::{Read, Write};

    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    stream.write_all(request)?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}

fn wait_for_http(port: u16, request: &[u8]) -> String {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut last_error = None;
    while Instant::now() < deadline {
        match http_request(port, request) {
            Ok(response) => return response,
            Err(error) => last_error = Some(error),
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("server did not answer before deadline: {last_error:?}")
}

#[test]
fn version_prints_package_version() {
    let output = bin().arg("version").output().unwrap();
    assert!(output.status.success());
    let line = stdout(&output);
    let prefix = format!("workengine {}", env!("CARGO_PKG_VERSION"));
    assert!(line.starts_with(&prefix), "version line: {line}");
    if let Some(rest) = line.strip_prefix(&prefix)
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
fn read_commands_do_not_recover_running_work() {
    let directory = tempfile::tempdir().unwrap();
    let id = create_work(directory.path());
    force_running(directory.path(), &id);

    for command in ["list", "show", "events"] {
        let mut invocation = bin();
        invocation.args(["--data-dir", directory.path().to_str().unwrap(), command]);
        if command == "show" {
            invocation.args(["--work", &id]);
        }
        let output = invocation.output().unwrap();
        assert!(
            output.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            status_and_event_count(directory.path(), &id),
            ("running".to_owned(), 1)
        );
    }
}

#[test]
fn serve_exposes_embedded_health_without_recovery() {
    use std::net::TcpListener;

    let directory = tempfile::tempdir().unwrap();
    let id = create_work(directory.path());
    force_running(directory.path(), &id);
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("reserve localhost port: {error}"),
    };
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut server = bin()
        .args([
            "--data-dir",
            directory.path().to_str().unwrap(),
            "serve",
            "--port",
            &port.to_string(),
        ])
        .spawn()
        .unwrap();
    let response = wait_for_http(
        port,
        b"GET /api/v0/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    let _ = server.kill();
    let _ = server.wait();
    assert!(response.contains("200 OK"), "response: {response}");
    assert!(
        response.contains("\"status\":\"ok\""),
        "response: {response}"
    );
    assert_eq!(
        status_and_event_count(directory.path(), &id),
        ("running".to_owned(), 1)
    );
}

#[test]
fn serve_starts_work_through_the_local_control_route() {
    use std::net::TcpListener;

    let directory = tempfile::tempdir().unwrap();
    let id = create_work(directory.path());
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("reserve localhost port: {error}"),
    };
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut server = bin()
        .args([
            "--data-dir",
            directory.path().to_str().unwrap(),
            "serve",
            "--port",
            &port.to_string(),
        ])
        .spawn()
        .unwrap();
    let health = wait_for_http(
        port,
        b"GET /api/v0/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert!(health.contains("200 OK"), "response: {health}");
    let request = format!(
        "POST /api/v0/works/{id}/start HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let response = http_request(port, request.as_bytes()).unwrap();
    let observation_request = format!(
        "GET /api/v0/works/{id}/observation HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    );
    let observation = http_request(port, observation_request.as_bytes()).unwrap();
    let _ = server.kill();
    let _ = server.wait();
    assert!(response.contains("200 OK"), "response: {response}");
    assert!(
        response.contains("\"status\":\"succeeded\""),
        "response: {response}"
    );
    assert!(
        observation.contains("\"runtimeKind\":\"stub\""),
        "observation: {observation}"
    );
    assert!(
        observation.contains("\"event\":\"child_stdout\"")
            && observation.contains("\"payloadRedacted\":true")
            && observation.contains("\"code\":\"confirmed_success\""),
        "observation: {observation}"
    );
    assert_eq!(
        status_and_event_count(directory.path(), &id),
        ("succeeded".to_owned(), 3)
    );
}

#[test]
fn start_rejects_shared_artifact_left_by_a_prior_run() {
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
    assert_eq!(second.status.code(), Some(30));
}

#[test]
fn no_args_is_usage_error() {
    let output = bin().output().unwrap();
    assert_eq!(output.status.code(), Some(2));
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
fn independent_create_is_not_blocked_by_a_running_work() {
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
    assert!(saw_spawned, "start never emitted spawned");
    let duplicate = bin()
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
        duplicate.status.code(),
        Some(10),
        "duplicate start stderr: {}",
        String::from_utf8_lossy(&duplicate.stderr)
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
    assert!(
        output.status.success(),
        "independent create stderr: {}",
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
fn process_profile_without_sandbox_is_rejected() {
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
    assert!(!start.status.success());
    assert!(String::from_utf8_lossy(&start.stderr).contains("must declare a sandbox"));
}

#[cfg(unix)]
#[test]
fn process_profile_without_sandbox_cannot_start() {
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
    assert_eq!(start.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&start.stderr).contains("must declare a sandbox"));
}

#[cfg(unix)]
#[test]
fn checkout_copy_requires_a_sandboxed_profile() {
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
    assert!(!start.status.success());
    assert!(String::from_utf8_lossy(&start.stderr).contains("must declare a sandbox"));
}
