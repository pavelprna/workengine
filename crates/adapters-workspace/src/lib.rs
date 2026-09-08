//! Dedicated workspace directory for the life of one Work.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use workengine_application::{AppError, WorkspaceFactory};
use workengine_domain::{OutcomeKind, WorkId, WorkStatus};

const MEMORY_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryLine {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    #[serde(rename = "workId")]
    work_id: String,
    status: String,
    #[serde(rename = "outcomeKind")]
    outcome_kind: String,
}

pub struct DirWorkspaceFactory {
    root: PathBuf,
}

impl DirWorkspaceFactory {
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        Self {
            root: data_dir.as_ref().join("workspaces"),
        }
    }

    fn dir(&self, work_id: &WorkId) -> PathBuf {
        self.root.join(work_id.as_str())
    }
}

impl WorkspaceFactory for DirWorkspaceFactory {
    fn bind(&self, work_id: &WorkId) -> Result<PathBuf, AppError> {
        let path = self.dir(work_id);
        fs::create_dir_all(&path).map_err(AppError::workspace)?;
        Ok(path)
    }

    fn read_artifact(&self, work_id: &WorkId) -> Result<Option<Vec<u8>>, AppError> {
        let path = self.dir(work_id).join("outcome.json");
        match fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(AppError::workspace(err)),
        }
    }

    fn record_memory(
        &self,
        work_id: &WorkId,
        status: WorkStatus,
        outcome_kind: OutcomeKind,
    ) -> Result<(), AppError> {
        let path = self.dir(work_id).join("memory.log");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(AppError::workspace)?;
        }
        let line = encode_memory(work_id, status, outcome_kind)?;
        if last_line(&path)?.as_deref() == Some(line.as_str()) {
            return Ok(());
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(AppError::workspace)?;
        writeln!(file, "{line}").map_err(AppError::workspace)?;
        Ok(())
    }
}

fn encode_memory(
    work_id: &WorkId,
    status: WorkStatus,
    outcome_kind: OutcomeKind,
) -> Result<String, AppError> {
    let payload = MemoryLine {
        schema_version: MEMORY_SCHEMA_VERSION,
        work_id: work_id.to_string(),
        status: status.as_str().to_owned(),
        outcome_kind: outcome_kind.as_str().to_owned(),
    };
    serde_json::to_string(&payload).map_err(AppError::workspace)
}

fn last_line(path: &Path) -> Result<Option<String>, AppError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text.lines().next_back().map(str::to_owned)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(AppError::workspace(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use workengine_domain::WorkId;

    #[test]
    fn two_work_ids_do_not_share_a_root() {
        let dir = tempfile::tempdir().unwrap();
        let factory = DirWorkspaceFactory::new(dir.path());
        let a = WorkId::parse("work-a").unwrap();
        let b = WorkId::parse("work-b").unwrap();
        let pa = factory.bind(&a).unwrap();
        let pb = factory.bind(&b).unwrap();
        assert_ne!(pa, pb);
        assert!(pa.starts_with(dir.path().join("workspaces")));
        assert!(pb.starts_with(dir.path().join("workspaces")));
        factory
            .record_memory(&a, WorkStatus::Failed, OutcomeKind::Failed)
            .unwrap();
        factory
            .record_memory(&a, WorkStatus::Succeeded, OutcomeKind::Succeeded)
            .unwrap();
        let memory = fs::read_to_string(pa.join("memory.log")).unwrap();
        let lines: Vec<_> = memory.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(!pb.join("memory.log").exists());
    }

    #[test]
    fn record_memory_does_not_duplicate_the_last_line() {
        let dir = tempfile::tempdir().unwrap();
        let factory = DirWorkspaceFactory::new(dir.path());
        let a = WorkId::parse("work-a").unwrap();
        factory.bind(&a).unwrap();
        factory
            .record_memory(&a, WorkStatus::Succeeded, OutcomeKind::Succeeded)
            .unwrap();
        factory
            .record_memory(&a, WorkStatus::Succeeded, OutcomeKind::Succeeded)
            .unwrap();
        let memory = fs::read_to_string(factory.dir(&a).join("memory.log")).unwrap();
        assert_eq!(memory.lines().count(), 1);
    }

    #[test]
    fn memory_schema_file_matches_closed_kinds() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../../schemas/memory.json")).unwrap();
        let statuses: Vec<&str> = schema["properties"]["status"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(statuses, vec!["succeeded", "failed"]);
        let kinds: Vec<&str> = schema["properties"]["outcomeKind"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let rust: Vec<&str> = OutcomeKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(kinds, rust);
        assert_eq!(schema["properties"]["schemaVersion"]["const"], 1);
    }
}
