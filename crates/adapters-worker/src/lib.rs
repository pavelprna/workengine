//! Stub Worker: a real process that writes a schema-valid outcome.

use std::fs;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use workengine_application::{AppError, RunRequest, WorkerRunner};
use workengine_domain::{OUTCOME_SCHEMA_VERSION, Outcome, OutcomeKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StubBehavior {
    Succeed,
    Fail,
    Hang,
}

#[derive(Clone, Debug)]
pub struct StubWorkerRunner {
    behavior: StubBehavior,
}

impl StubWorkerRunner {
    pub fn new(behavior: StubBehavior) -> Self {
        Self { behavior }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutcomeDto {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    kind: String,
    #[serde(rename = "workerProfile")]
    worker_profile: String,
}

impl WorkerRunner for StubWorkerRunner {
    fn run(&self, request: &RunRequest<'_>) -> Result<Outcome, AppError> {
        let profile = request.work.attributes().worker_profile();
        match self.behavior {
            StubBehavior::Hang => run_hang(request, profile),
            StubBehavior::Succeed => run_writer(request, OutcomeKind::Succeeded, profile),
            StubBehavior::Fail => run_writer(request, OutcomeKind::Failed, profile),
        }
    }

    fn decode(&self, bytes: &[u8]) -> Result<Outcome, AppError> {
        decode_outcome(bytes)
    }
}

pub fn decode_outcome(bytes: &[u8]) -> Result<Outcome, AppError> {
    let dto: OutcomeDto =
        serde_json::from_str(std::str::from_utf8(bytes).map_err(AppError::outcome_schema)?)
            .map_err(AppError::outcome_schema)?;
    let kind: OutcomeKind = dto.kind.parse().map_err(AppError::outcome_schema)?;
    Outcome::new(dto.schema_version, kind, dto.worker_profile).map_err(|err| match err {
        workengine_domain::DomainError::UnknownOutcomeKind(_)
        | workengine_domain::DomainError::UnsupportedSchemaVersion(_) => {
            AppError::outcome_schema(err)
        }
        other => AppError::from(other),
    })
}

pub fn encode_outcome(outcome: &Outcome) -> Result<String, AppError> {
    let dto = OutcomeDto {
        schema_version: outcome.schema_version(),
        kind: outcome.kind().as_str().to_owned(),
        worker_profile: outcome.worker_profile().to_owned(),
    };
    serde_json::to_string(&dto).map_err(AppError::worker)
}

fn run_writer(
    request: &RunRequest<'_>,
    kind: OutcomeKind,
    profile: &str,
) -> Result<Outcome, AppError> {
    fs::create_dir_all(request.workspace_root).map_err(AppError::worker)?;
    let outcome = Outcome::new(OUTCOME_SCHEMA_VERSION, kind, profile).map_err(AppError::from)?;
    let json = encode_outcome(&outcome)?;
    let dest = request.workspace_root.join("outcome.json");
    let script = format!("printf '%s\\n' '{json}' > '{}'", dest.display());
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(script)
        .current_dir(request.workspace_root)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    apply_process_group(&mut cmd);
    let mut child = cmd.spawn().map_err(AppError::worker)?;
    wait_child(&mut child, request.budget)?;
    decode_outcome(&fs::read(&dest).map_err(AppError::worker)?)
}

fn wait_child(child: &mut std::process::Child, budget: Duration) -> Result<(), AppError> {
    let deadline = Instant::now() + budget;
    loop {
        match child.try_wait().map_err(AppError::worker)? {
            Some(status) if status.success() => return Ok(()),
            Some(status) => {
                return Err(AppError::worker(format!("stub worker exited {status}")));
            }
            None if Instant::now() >= deadline => {
                kill_group(child.id());
                let _ = child.wait();
                return Err(AppError::worker("stub worker exceeded budget"));
            }
            None => thread::sleep(Duration::from_millis(5)),
        }
    }
}

fn run_hang(request: &RunRequest<'_>, profile: &str) -> Result<Outcome, AppError> {
    fs::create_dir_all(request.workspace_root).map_err(AppError::worker)?;
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg("sleep 30 & echo $! > child.pid; exec sleep 30")
        .current_dir(request.workspace_root)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    apply_process_group(&mut cmd);
    let mut child = cmd.spawn().map_err(AppError::worker)?;
    let deadline = Instant::now() + request.budget;
    loop {
        match child.try_wait().map_err(AppError::worker)? {
            Some(_) => break,
            None if Instant::now() >= deadline => {
                kill_group(child.id());
                let _ = child.wait();
                let outcome = Outcome::new(OUTCOME_SCHEMA_VERSION, OutcomeKind::TimedOut, profile)?;
                let json = encode_outcome(&outcome)?;
                fs::write(request.workspace_root.join("outcome.json"), json)
                    .map_err(AppError::worker)?;
                return Ok(outcome);
            }
            None => thread::sleep(Duration::from_millis(10)),
        }
    }
    Err(AppError::worker("hang stub exited before budget"))
}

fn apply_process_group(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
}

fn kill_group(pid: u32) {
    #[cfg(unix)]
    {
        use nix::sys::signal::{self, Signal};
        use nix::unistd::Pid;
        let pid = Pid::from_raw(pid as i32);
        let _ = signal::killpg(pid, Signal::SIGTERM);
        thread::sleep(Duration::from_millis(50));
        let _ = signal::killpg(pid, Signal::SIGKILL);
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use workengine_domain::{Work, WorkAttributes, WorkId};

    fn work() -> Work {
        Work::new(
            WorkId::parse("work-1").unwrap(),
            WorkAttributes::new("goal", "stub").unwrap(),
            1,
        )
        .unwrap()
    }

    #[test]
    fn stub_writes_schema_valid_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let runner = StubWorkerRunner::new(StubBehavior::Succeed);
        let work = work();
        let outcome = runner
            .run(&RunRequest {
                work: &work,
                workspace_root: dir.path(),
                budget: Duration::from_secs(2),
            })
            .unwrap();
        assert_eq!(outcome.kind(), OutcomeKind::Succeeded);
        decode_outcome(&fs::read(dir.path().join("outcome.json")).unwrap()).unwrap();
    }

    #[test]
    fn unknown_kind_is_schema_error() {
        let runner = StubWorkerRunner::new(StubBehavior::Succeed);
        let err = runner
            .decode(br#"{"schemaVersion":1,"kind":"needs_review","workerProfile":"stub"}"#)
            .unwrap_err();
        assert!(matches!(err, AppError::OutcomeSchema(_)));
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_the_process_group() {
        use nix::sys::signal::kill;
        use nix::unistd::Pid;

        let dir = tempfile::tempdir().unwrap();
        let runner = StubWorkerRunner::new(StubBehavior::Hang);
        let work = work();
        let outcome = runner
            .run(&RunRequest {
                work: &work,
                workspace_root: dir.path(),
                budget: Duration::from_millis(80),
            })
            .unwrap();
        assert_eq!(outcome.kind(), OutcomeKind::TimedOut);
        let child_pid: i32 = fs::read_to_string(dir.path().join("child.pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        thread::sleep(Duration::from_millis(20));
        let alive = kill(Pid::from_raw(child_pid), None);
        assert!(alive.is_err(), "grandchild sleep should be dead: {alive:?}");
    }
}
