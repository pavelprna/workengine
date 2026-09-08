//! Generic process Worker: argv + env from a profile. Not a vendor adapter.

use std::fs;
use std::path::Path;
use std::process::Command;

use workengine_application::{AppError, ChannelReaction, RunRequest, WorkerRunner};
use workengine_domain::{OUTCOME_SCHEMA_VERSION, Outcome, OutcomeKind};

use crate::decode_outcome;
use crate::encode_outcome;
use crate::supervise::{ChildWait, spawn_supervised};

pub struct ProcessWorkerRunner {
    argv: Vec<String>,
    env: Vec<(String, String)>,
}

impl ProcessWorkerRunner {
    pub fn new(argv: Vec<String>, env: Vec<(String, String)>) -> Result<Self, AppError> {
        if argv.is_empty() || argv[0].is_empty() {
            return Err(AppError::worker("profile argv is empty"));
        }
        Ok(Self { argv, env })
    }
}

impl WorkerRunner for ProcessWorkerRunner {
    fn run(&self, request: &RunRequest<'_>) -> Result<Outcome, AppError> {
        let profile = request.work.attributes().worker_profile();
        fs::create_dir_all(request.workspace_root).map_err(AppError::worker)?;
        let mut cmd = Command::new(&self.argv[0]);
        cmd.args(&self.argv[1..]);
        cmd.current_dir(request.workspace_root);
        cmd.env_clear();
        for (key, value) in &self.env {
            cmd.env(key, value);
        }
        match spawn_supervised(cmd, request.work.id().as_str(), request.budget) {
            Ok(ChildWait::Exited { .. }) => read_or_fail(request.workspace_root, profile),
            Ok(ChildWait::BudgetExceeded) => persist_timed_out(request.workspace_root, profile),
            Err(_) => Err(AppError::Channel(ChannelReaction::Fail)),
        }
    }

    fn decode(&self, bytes: &[u8]) -> Result<Outcome, AppError> {
        decode_outcome(bytes)
    }
}

fn read_or_fail(root: &Path, profile: &str) -> Result<Outcome, AppError> {
    let dest = root.join("outcome.json");
    match fs::read(&dest) {
        Ok(bytes) => decode_outcome(&bytes),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Outcome::new(OUTCOME_SCHEMA_VERSION, OutcomeKind::Failed, profile)
                .map_err(AppError::from)
        }
        Err(err) => Err(AppError::worker(err)),
    }
}

fn persist_timed_out(root: &Path, profile: &str) -> Result<Outcome, AppError> {
    let outcome = Outcome::new(OUTCOME_SCHEMA_VERSION, OutcomeKind::TimedOut, profile)
        .map_err(AppError::from)?;
    let json = encode_outcome(&outcome)?;
    fs::write(root.join("outcome.json"), json).map_err(AppError::worker)?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use workengine_domain::{Work, WorkAttributes, WorkId};

    fn work() -> Work {
        Work::new(
            WorkId::parse("work-1").unwrap(),
            WorkAttributes::new("goal", "echo").unwrap(),
            1,
        )
        .unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn missing_outcome_is_failed_not_succeeded() {
        let dir = tempfile::tempdir().unwrap();
        let runner = ProcessWorkerRunner::new(vec!["/bin/true".to_owned()], vec![]).unwrap();
        let work = work();
        let outcome = runner
            .run(&RunRequest {
                work: &work,
                workspace_root: dir.path(),
                budget: Duration::from_secs(2),
            })
            .unwrap();
        assert_eq!(outcome.kind(), OutcomeKind::Failed);
        assert!(!dir.path().join("outcome.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn schema_outcome_file_is_decoded() {
        let dir = tempfile::tempdir().unwrap();
        let runner = ProcessWorkerRunner::new(
            vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                "printf '%s\\n' '{\"schemaVersion\":1,\"kind\":\"succeeded\",\"workerProfile\":\"echo\"}' > outcome.json"
                    .to_owned(),
            ],
            vec![],
        )
        .unwrap();
        let work = work();
        let outcome = runner
            .run(&RunRequest {
                work: &work,
                workspace_root: dir.path(),
                budget: Duration::from_secs(2),
            })
            .unwrap();
        assert_eq!(outcome.kind(), OutcomeKind::Succeeded);
    }

    #[cfg(unix)]
    #[test]
    fn undeclared_env_is_not_inherited() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            std::env::var("HOME").is_ok(),
            "parent HOME must exist so the test can prove it is not inherited"
        );
        let runner = ProcessWorkerRunner::new(
            vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                "printenv HOME > leaked.txt || true".to_owned(),
            ],
            vec![],
        )
        .unwrap();
        let work = work();
        let _ = runner
            .run(&RunRequest {
                work: &work,
                workspace_root: dir.path(),
                budget: Duration::from_secs(2),
            })
            .unwrap();
        let leaked = fs::read_to_string(dir.path().join("leaked.txt")).unwrap();
        assert!(
            leaked.trim().is_empty(),
            "child inherited undeclared HOME: {leaked}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn declared_env_is_passed() {
        let dir = tempfile::tempdir().unwrap();
        let runner = ProcessWorkerRunner::new(
            vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                "printenv WORKENGINE_TEST_OK > ok.txt".to_owned(),
            ],
            vec![("WORKENGINE_TEST_OK".to_owned(), "visible".to_owned())],
        )
        .unwrap();
        let work = work();
        let _ = runner
            .run(&RunRequest {
                work: &work,
                workspace_root: dir.path(),
                budget: Duration::from_secs(2),
            })
            .unwrap();
        let got = fs::read_to_string(dir.path().join("ok.txt")).unwrap();
        assert_eq!(got.trim(), "visible");
    }
}
