//! Dedicated workspace directory for the life of one Work.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use workengine_application::{AppError, BindRequest, OperatorInput, WorkspaceFactory};
use workengine_domain::{AttemptId, ExecutionId, OutcomeKind, ProjectId, WorkId, WorkStatus};

const MEMORY_SCHEMA_VERSION: u32 = 1;
pub const GOAL_FILE: &str = "workengine-goal.txt";

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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OperatorInputLine<'a> {
    schema_version: u32,
    input_id: i64,
    work_id: &'a str,
    kind: &'a str,
    body: &'a str,
    created_at_unix_ms: u64,
}

pub struct DirWorkspaceFactory {
    root: PathBuf,
    control_root: PathBuf,
}

impl DirWorkspaceFactory {
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        Self {
            root: data_dir.as_ref().join("workspaces"),
            control_root: data_dir.as_ref().join("control"),
        }
    }

    fn dir(&self, project_id: &ProjectId, work_id: &WorkId) -> PathBuf {
        let scoped = self.root.join(project_id.as_str()).join(work_id.as_str());
        let legacy = self.root.join(work_id.as_str());
        if project_id == &ProjectId::default_project() && legacy.exists() && !scoped.exists() {
            legacy
        } else {
            scoped
        }
    }
}

impl WorkspaceFactory for DirWorkspaceFactory {
    fn bind(&self, request: &BindRequest<'_>) -> Result<PathBuf, AppError> {
        let project_path = self.root.join(request.project_id.as_str());
        reject_symlink(&project_path, "project workspace path")?;
        fs::create_dir_all(&project_path).map_err(AppError::workspace)?;
        restrict_directory(&project_path)?;
        let path = self.dir(request.project_id, request.work_id);
        if path
            .symlink_metadata()
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(AppError::workspace("workspace path must not be a symlink"));
        }
        let existed = path.exists();
        fs::create_dir_all(&path).map_err(AppError::workspace)?;
        restrict_directory(&path)?;
        if !existed && let Some(checkout) = request.checkout {
            copy_tree(checkout, &path)?;
        }
        write_no_follow(&path.join(GOAL_FILE), request.goal.as_bytes())?;
        Ok(path)
    }

    fn read_artifact(
        &self,
        work_id: &WorkId,
        project_id: &ProjectId,
    ) -> Result<Option<Vec<u8>>, AppError> {
        let path = self.dir(project_id, work_id).join("outcome.json");
        match read_no_follow(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(AppError::workspace(err)),
        }
    }

    fn record_memory(
        &self,
        work_id: &WorkId,
        project_id: &ProjectId,
        status: WorkStatus,
        outcome_kind: OutcomeKind,
    ) -> Result<(), AppError> {
        let path = self.dir(project_id, work_id).join("memory.log");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(AppError::workspace)?;
        }
        let line = encode_memory(work_id, status, outcome_kind)?;
        if last_line(&path)?.as_deref() == Some(line.as_str()) {
            return Ok(());
        }
        let mut file = append_no_follow(&path)?;
        writeln!(file, "{line}").map_err(AppError::workspace)?;
        Ok(())
    }

    fn bind_attempt_control(
        &self,
        work_id: &WorkId,
        project_id: &ProjectId,
        execution_id: &ExecutionId,
        attempt_id: &AttemptId,
    ) -> Result<PathBuf, AppError> {
        let path = self
            .control_root
            .join(project_id.as_str())
            .join(work_id.as_str())
            .join(execution_id.as_str())
            .join(attempt_id.as_str());
        if path
            .symlink_metadata()
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(AppError::workspace(
                "attempt control path must not be a symlink",
            ));
        }
        fs::create_dir_all(&path).map_err(AppError::workspace)?;
        restrict_directory(&path)?;
        Ok(path)
    }

    fn record_operator_input(
        &self,
        work_id: &WorkId,
        project_id: &ProjectId,
        input: &OperatorInput,
    ) -> Result<(), AppError> {
        let directory = self.dir(project_id, work_id).join(".workengine");
        reject_symlink(&directory, "operator-input directory")?;
        fs::create_dir_all(&directory).map_err(AppError::workspace)?;
        restrict_directory(&directory)?;
        let path = directory.join("operator-inputs.jsonl");
        let line = serde_json::to_string(&OperatorInputLine {
            schema_version: 1,
            input_id: input.id,
            work_id: work_id.as_str(),
            kind: input.kind.as_str(),
            body: &input.body,
            created_at_unix_ms: input.created_at_unix_ms,
        })
        .map_err(AppError::workspace)?;
        if last_line(&path)?.as_deref() == Some(line.as_str()) {
            return Ok(());
        }
        let mut file = append_no_follow(&path)?;
        writeln!(file, "{line}").map_err(AppError::workspace)
    }
}

fn restrict_directory(path: &Path) -> Result<(), AppError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(AppError::workspace)?;
    }
    Ok(())
}

fn reject_symlink(path: &Path, label: &str) -> Result<(), AppError> {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(AppError::workspace(format!(
            "{label} must not be a symlink"
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::workspace(error)),
    }
}

fn no_follow(options: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK);
    }
}

fn write_no_follow(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    no_follow(&mut options);
    let mut file = options.open(path).map_err(AppError::workspace)?;
    if !file.metadata().map_err(AppError::workspace)?.is_file() {
        return Err(AppError::workspace(
            "workspace control path is not a regular file",
        ));
    }
    file.write_all(bytes).map_err(AppError::workspace)?;
    file.sync_all().map_err(AppError::workspace)
}

fn append_no_follow(path: &Path) -> Result<std::fs::File, AppError> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).append(true);
    no_follow(&mut options);
    let file = options.open(path).map_err(AppError::workspace)?;
    if !file.metadata().map_err(AppError::workspace)?.is_file() {
        return Err(AppError::workspace(
            "workspace control path is not a regular file",
        ));
    }
    Ok(file)
}

fn read_no_follow(path: &Path) -> Result<Vec<u8>, std::io::Error> {
    use std::io::Read;

    let mut options = OpenOptions::new();
    options.read(true);
    no_follow(&mut options);
    let mut file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "workspace control path is not a regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn copy_tree(src: &Path, dest: &Path) -> Result<(), AppError> {
    if !src.is_dir() {
        return Err(AppError::workspace(format!(
            "checkout is not a directory: {}",
            src.display()
        )));
    }
    let src = src.canonicalize().map_err(AppError::workspace)?;
    let dest = dest.canonicalize().map_err(AppError::workspace)?;
    if src == dest {
        return Err(AppError::workspace("checkout is the workspace root"));
    }
    copy_dir(&src, &dest, &dest)
}

fn copy_dir(src: &Path, dest: &Path, skip: &Path) -> Result<(), AppError> {
    fs::create_dir_all(dest).map_err(AppError::workspace)?;
    for entry in fs::read_dir(src).map_err(AppError::workspace)? {
        let entry = entry.map_err(AppError::workspace)?;
        let from = entry.path();
        if from == *skip || skip.starts_with(&from) {
            continue;
        }
        let ty = entry.file_type().map_err(AppError::workspace)?;
        if ty.is_symlink() {
            continue;
        }
        let to = dest.join(entry.file_name());
        if ty.is_dir() {
            copy_dir(&from, &to, skip)?;
        } else if ty.is_file() {
            fs::copy(from, &to).map_err(AppError::workspace)?;
        }
    }
    Ok(())
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
    match read_no_follow(path) {
        Ok(bytes) => {
            let text = String::from_utf8(bytes).map_err(AppError::workspace)?;
            Ok(text.lines().next_back().map(str::to_owned))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(AppError::workspace(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use workengine_application::{OperatorInput, OperatorInputKind};
    use workengine_domain::WorkId;

    fn bind(
        factory: &DirWorkspaceFactory,
        id: &WorkId,
        checkout: Option<&Path>,
    ) -> Result<PathBuf, AppError> {
        let project = ProjectId::default_project();
        factory.bind(&BindRequest {
            work_id: id,
            project_id: &project,
            goal: "do the thing",
            checkout,
        })
    }

    #[test]
    fn two_work_ids_do_not_share_a_root() {
        let dir = tempfile::tempdir().unwrap();
        let factory = DirWorkspaceFactory::new(dir.path());
        let a = WorkId::parse("work-a").unwrap();
        let b = WorkId::parse("work-b").unwrap();
        let pa = bind(&factory, &a, None).unwrap();
        let pb = bind(&factory, &b, None).unwrap();
        assert_ne!(pa, pb);
        assert!(pa.starts_with(dir.path().join("workspaces")));
        assert!(pb.starts_with(dir.path().join("workspaces")));
        factory
            .record_memory(
                &a,
                &ProjectId::default_project(),
                WorkStatus::Failed,
                OutcomeKind::Failed,
            )
            .unwrap();
        factory
            .record_memory(
                &a,
                &ProjectId::default_project(),
                WorkStatus::Succeeded,
                OutcomeKind::Succeeded,
            )
            .unwrap();
        let memory = fs::read_to_string(pa.join("memory.log")).unwrap();
        let lines: Vec<_> = memory.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(!pb.join("memory.log").exists());
    }

    #[test]
    fn projects_do_not_share_repository_roots_even_for_the_same_work_id() {
        let dir = tempfile::tempdir().unwrap();
        let factory = DirWorkspaceFactory::new(dir.path());
        let id = WorkId::parse("work-a").unwrap();
        let project_a = ProjectId::parse("project-a").unwrap();
        let project_b = ProjectId::parse("project-b").unwrap();
        let a = factory
            .bind(&BindRequest {
                work_id: &id,
                project_id: &project_a,
                goal: "a",
                checkout: None,
            })
            .unwrap();
        let b = factory
            .bind(&BindRequest {
                work_id: &id,
                project_id: &project_b,
                goal: "b",
                checkout: None,
            })
            .unwrap();
        assert_ne!(a, b);
        assert!(a.starts_with(dir.path().join("workspaces/project-a")));
        assert!(b.starts_with(dir.path().join("workspaces/project-b")));
    }

    #[test]
    fn default_project_reuses_a_legacy_unscoped_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let factory = DirWorkspaceFactory::new(dir.path());
        let id = WorkId::parse("legacy").unwrap();
        let legacy = dir.path().join("workspaces/legacy");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("kept.txt"), "kept").unwrap();
        let rebound = bind(&factory, &id, None).unwrap();
        assert_eq!(rebound, legacy);
        assert_eq!(
            fs::read_to_string(rebound.join("kept.txt")).unwrap(),
            "kept"
        );
    }

    #[test]
    fn record_memory_does_not_duplicate_the_last_line() {
        let dir = tempfile::tempdir().unwrap();
        let factory = DirWorkspaceFactory::new(dir.path());
        let a = WorkId::parse("work-a").unwrap();
        bind(&factory, &a, None).unwrap();
        factory
            .record_memory(
                &a,
                &ProjectId::default_project(),
                WorkStatus::Succeeded,
                OutcomeKind::Succeeded,
            )
            .unwrap();
        factory
            .record_memory(
                &a,
                &ProjectId::default_project(),
                WorkStatus::Succeeded,
                OutcomeKind::Succeeded,
            )
            .unwrap();
        let memory = fs::read_to_string(
            factory
                .dir(&ProjectId::default_project(), &a)
                .join("memory.log"),
        )
        .unwrap();
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

    #[test]
    fn bind_writes_the_goal_as_data() {
        let dir = tempfile::tempdir().unwrap();
        let factory = DirWorkspaceFactory::new(dir.path());
        let a = WorkId::parse("work-a").unwrap();
        let path = bind(&factory, &a, None).unwrap();
        assert_eq!(
            fs::read_to_string(path.join(GOAL_FILE)).unwrap(),
            "do the thing"
        );
    }

    #[test]
    fn bind_copies_checkout_once() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("hello.txt"), "from-source").unwrap();
        let factory = DirWorkspaceFactory::new(dir.path());
        let a = WorkId::parse("work-a").unwrap();
        let path = bind(&factory, &a, Some(&src)).unwrap();
        assert_eq!(
            fs::read_to_string(path.join("hello.txt")).unwrap(),
            "from-source"
        );
        fs::write(path.join("hello.txt"), "worker-edit").unwrap();
        fs::write(src.join("hello.txt"), "source-changed").unwrap();
        let again = bind(&factory, &a, Some(&src)).unwrap();
        assert_eq!(again, path);
        assert_eq!(
            fs::read_to_string(path.join("hello.txt")).unwrap(),
            "worker-edit"
        );
    }

    #[cfg(unix)]
    #[test]
    fn copy_skips_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("real.txt"), "ok").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", src.join("link")).unwrap();
        let factory = DirWorkspaceFactory::new(dir.path());
        let a = WorkId::parse("work-a").unwrap();
        let path = bind(&factory, &a, Some(&src)).unwrap();
        assert!(path.join("real.txt").exists());
        assert!(!path.join("link").exists());
    }

    #[cfg(unix)]
    #[test]
    fn control_plane_files_do_not_follow_worker_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside.txt");
        fs::write(&outside, "protected").unwrap();
        let factory = DirWorkspaceFactory::new(dir.path());
        let id = WorkId::parse("work-links").unwrap();
        let root = bind(&factory, &id, None).unwrap();

        std::os::unix::fs::symlink(&outside, root.join("outcome.json")).unwrap();
        assert!(
            factory
                .read_artifact(&id, &ProjectId::default_project())
                .is_err()
        );

        std::os::unix::fs::symlink(&outside, root.join("memory.log")).unwrap();
        assert!(
            factory
                .record_memory(
                    &id,
                    &ProjectId::default_project(),
                    WorkStatus::Failed,
                    OutcomeKind::Failed,
                )
                .is_err()
        );
        assert_eq!(fs::read_to_string(&outside).unwrap(), "protected");
    }

    #[cfg(unix)]
    #[test]
    fn operator_input_directory_cannot_be_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let factory = DirWorkspaceFactory::new(dir.path());
        let id = WorkId::parse("work-input-link").unwrap();
        let root = bind(&factory, &id, None).unwrap();
        std::os::unix::fs::symlink("/tmp", root.join(".workengine")).unwrap();
        let input = OperatorInput {
            id: 1,
            kind: OperatorInputKind::Answer,
            body: "data".to_owned(),
            created_at_unix_ms: 1,
        };
        assert!(
            factory
                .record_operator_input(&id, &ProjectId::default_project(), &input)
                .is_err()
        );
    }

    #[test]
    fn operator_inputs_are_append_only_data_for_the_same_work() {
        let dir = tempfile::tempdir().unwrap();
        let factory = DirWorkspaceFactory::new(dir.path());
        let id = WorkId::parse("work-input").unwrap();
        bind(&factory, &id, None).unwrap();
        let input = OperatorInput {
            id: 7,
            kind: OperatorInputKind::Answer,
            body: "choose B".to_owned(),
            created_at_unix_ms: 42,
        };
        factory
            .record_operator_input(&id, &ProjectId::default_project(), &input)
            .unwrap();
        factory
            .record_operator_input(&id, &ProjectId::default_project(), &input)
            .unwrap();
        let text = fs::read_to_string(
            factory
                .dir(&ProjectId::default_project(), &id)
                .join(".workengine/operator-inputs.jsonl"),
        )
        .unwrap();
        assert_eq!(text.lines().count(), 1);
        let value: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(value["workId"], "work-input");
        assert_eq!(value["kind"], "answer");
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../../schemas/operator-input.json")).unwrap();
        assert_eq!(schema["properties"]["schemaVersion"]["const"], 1);
    }
}
