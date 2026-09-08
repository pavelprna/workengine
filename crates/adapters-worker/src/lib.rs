//! Stub Worker: a real process that writes a schema-valid outcome.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use workengine_application::{AppError, ChannelReaction, RunRequest, WorkerRunner};
use workengine_domain::{OUTCOME_SCHEMA_VERSION, Outcome, OutcomeKind};

mod process;
mod stream;
mod supervise;

pub use process::ProcessWorkerRunner;
pub use stream::{
    EVENT_CHILD_STDERR, EVENT_CHILD_STDOUT, EVENT_EXITED, EVENT_KILLED, EVENT_SPAWNED,
    STREAM_EVENTS, STREAM_SCHEMA_VERSION, record_line,
};
use supervise::{ChildWait, spawn_supervised};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StubBehavior {
    Succeed,
    Fail,
    Hang,
    ExceedBudget,
    ChannelFail,
    ChannelPark,
    ChannelRetry,
}

impl FromStr for StubBehavior {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "succeed" => Ok(Self::Succeed),
            "fail" => Ok(Self::Fail),
            "hang" => Ok(Self::Hang),
            "exceed_budget" => Ok(Self::ExceedBudget),
            "channel_fail" => Ok(Self::ChannelFail),
            "channel_park" => Ok(Self::ChannelPark),
            "channel_retry" => Ok(Self::ChannelRetry),
            other => Err(format!("unknown stub behavior {other}")),
        }
    }
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
            StubBehavior::ExceedBudget => run_budget(request, profile),
            StubBehavior::ChannelFail => Err(AppError::Channel(ChannelReaction::Fail)),
            StubBehavior::ChannelPark => Err(AppError::Channel(ChannelReaction::Park)),
            StubBehavior::ChannelRetry => Err(AppError::Channel(ChannelReaction::Retry)),
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
    let payload = request.workspace_root.join(".outcome-payload.json");
    fs::write(&payload, &json).map_err(AppError::worker)?;
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg("cp .outcome-payload.json outcome.json && printf '%s\\n' stub")
        .current_dir(request.workspace_root);
    match spawn_supervised(cmd, request.work.id().as_str(), request.budget)? {
        ChildWait::Exited { success: true } => {
            decode_outcome(&fs::read(&dest).map_err(AppError::worker)?)
        }
        ChildWait::Exited { success: false } => {
            Err(AppError::worker("stub worker exited unsuccessfully"))
        }
        ChildWait::BudgetExceeded => {
            persist_outcome(request.workspace_root, OutcomeKind::BudgetExceeded, profile)
        }
    }
}

fn persist_outcome(root: &Path, kind: OutcomeKind, profile: &str) -> Result<Outcome, AppError> {
    let outcome = Outcome::new(OUTCOME_SCHEMA_VERSION, kind, profile).map_err(AppError::from)?;
    let json = encode_outcome(&outcome)?;
    fs::write(root.join("outcome.json"), json).map_err(AppError::worker)?;
    Ok(outcome)
}

fn run_budget(request: &RunRequest<'_>, profile: &str) -> Result<Outcome, AppError> {
    fs::create_dir_all(request.workspace_root).map_err(AppError::worker)?;
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg("exec sleep 30")
        .current_dir(request.workspace_root);
    match spawn_supervised(cmd, request.work.id().as_str(), request.budget)? {
        ChildWait::BudgetExceeded => {
            persist_outcome(request.workspace_root, OutcomeKind::BudgetExceeded, profile)
        }
        ChildWait::Exited { .. } => Err(AppError::worker("budget stub exited before budget")),
    }
}

fn run_hang(request: &RunRequest<'_>, profile: &str) -> Result<Outcome, AppError> {
    fs::create_dir_all(request.workspace_root).map_err(AppError::worker)?;
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg("sleep 30 & echo $! > child.pid; exec sleep 30")
        .current_dir(request.workspace_root);
    match spawn_supervised(cmd, request.work.id().as_str(), request.budget)? {
        ChildWait::BudgetExceeded => {
            persist_outcome(request.workspace_root, OutcomeKind::TimedOut, profile)
        }
        ChildWait::Exited { .. } => Err(AppError::worker("hang stub exited before budget")),
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
        use std::thread;

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

    #[test]
    fn writer_timeout_is_budget_exceeded_not_channel_error() {
        let dir = tempfile::tempdir().unwrap();
        let runner = StubWorkerRunner::new(StubBehavior::ExceedBudget);
        let work = work();
        let outcome = runner
            .run(&RunRequest {
                work: &work,
                workspace_root: dir.path(),
                budget: Duration::from_millis(80),
            })
            .unwrap();
        assert_eq!(outcome.kind(), OutcomeKind::BudgetExceeded);
        let loaded = decode_outcome(&fs::read(dir.path().join("outcome.json")).unwrap()).unwrap();
        assert_eq!(loaded.kind(), OutcomeKind::BudgetExceeded);
    }

    #[test]
    fn outcome_schema_file_matches_closed_kinds() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../../schemas/outcome.json")).unwrap();
        let kinds: Vec<&str> = schema["properties"]["kind"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let rust: Vec<&str> = OutcomeKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(kinds, rust);
        assert_eq!(schema["properties"]["schemaVersion"]["const"], 1);
    }

    #[test]
    fn stream_line_carries_work_id_schema_version_and_event() {
        let line = record_line("work-1", EVENT_SPAWNED, None);
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["schemaVersion"], STREAM_SCHEMA_VERSION);
        assert_eq!(v["workId"], "work-1");
        assert_eq!(v["event"], EVENT_SPAWNED);
        assert!(v.get("payload").is_none());
        let with_payload = record_line("work-1", EVENT_CHILD_STDOUT, Some("stub"));
        let v: serde_json::Value = serde_json::from_str(&with_payload).unwrap();
        assert_eq!(v["payload"], "stub");
        assert_eq!(v["workId"], "work-1");
    }

    #[test]
    fn log_schema_file_matches_stream_events() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../../schemas/log.json")).unwrap();
        let events: Vec<&str> = schema["properties"]["event"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(events, STREAM_EVENTS);
        assert_eq!(schema["properties"]["schemaVersion"]["const"], 1);
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(required.contains(&"workId"));
        assert!(required.contains(&"event"));
        assert!(required.contains(&"schemaVersion"));
    }

    #[test]
    fn writer_survives_quotes_in_the_worker_profile() {
        let dir = tempfile::tempdir().unwrap();
        let runner = StubWorkerRunner::new(StubBehavior::Succeed);
        let work = Work::new(
            WorkId::parse("work-1").unwrap(),
            WorkAttributes::new("goal", "stub's \"quoted\"").unwrap(),
            1,
        )
        .unwrap();
        let outcome = runner
            .run(&RunRequest {
                work: &work,
                workspace_root: dir.path(),
                budget: Duration::from_secs(2),
            })
            .unwrap();
        assert_eq!(outcome.kind(), OutcomeKind::Succeeded);
        assert_eq!(outcome.worker_profile(), "stub's \"quoted\"");
        let loaded = decode_outcome(&fs::read(dir.path().join("outcome.json")).unwrap()).unwrap();
        assert_eq!(loaded.worker_profile(), "stub's \"quoted\"");
    }

    #[test]
    fn channel_behaviors_are_classified_not_prose() {
        let dir = tempfile::tempdir().unwrap();
        let work = work();
        let request = RunRequest {
            work: &work,
            workspace_root: dir.path(),
            budget: Duration::from_secs(1),
        };
        let fail = StubWorkerRunner::new(StubBehavior::ChannelFail)
            .run(&request)
            .unwrap_err();
        assert!(matches!(err_channel(&fail), ChannelReaction::Fail));
        let park = StubWorkerRunner::new(StubBehavior::ChannelPark)
            .run(&request)
            .unwrap_err();
        assert!(matches!(err_channel(&park), ChannelReaction::Park));
        let retry = StubWorkerRunner::new(StubBehavior::ChannelRetry)
            .run(&request)
            .unwrap_err();
        assert!(matches!(err_channel(&retry), ChannelReaction::Retry));
    }

    fn err_channel(err: &AppError) -> ChannelReaction {
        match err {
            AppError::Channel(reaction) => *reaction,
            other => panic!("expected channel error, got {other}"),
        }
    }
}
