//! Dedicated workspace directory for the life of one Work.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use workengine_application::{AppError, WorkspaceFactory};
use workengine_domain::WorkId;

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

    fn record_memory(&self, work_id: &WorkId, entry: &str) -> Result<(), AppError> {
        let path = self.dir(work_id).join("memory.log");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(AppError::workspace)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(AppError::workspace)?;
        writeln!(file, "{entry}").map_err(AppError::workspace)?;
        Ok(())
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
        factory.record_memory(&a, "one").unwrap();
        factory.record_memory(&a, "two").unwrap();
        let memory = fs::read_to_string(pa.join("memory.log")).unwrap();
        assert_eq!(memory, "one\ntwo\n");
        assert!(!pb.join("memory.log").exists());
    }
}
