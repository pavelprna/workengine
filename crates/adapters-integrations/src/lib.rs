//! Product-neutral file adapters and isolated one-shot integration jobs.

use std::fs::{OpenOptions, read_to_string};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};

use serde::{Deserialize, Serialize};
use workengine_application::{
    AppError, InboundRecord, InboundSignal, InboundSource, InboundStore, Publication,
    PublicationStore, Publisher, SystemClock, dispatch_publications, poll_inbound,
};
use workengine_domain::{ProjectId, Work};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileInboundRecord {
    record_id: String,
    signal: String,
    project_id: String,
    repository: Option<String>,
    goal: String,
    worker_profile: String,
    notification_target: Option<String>,
}

/// JSON-lines inbox. Re-reading is safe because the control plane owns receipts.
pub struct JsonLinesInbound {
    source_id: String,
    path: PathBuf,
}

impl JsonLinesInbound {
    pub fn new(source_id: impl Into<String>, path: impl Into<PathBuf>) -> Result<Self, AppError> {
        let source_id = source_id.into();
        if source_id.trim().is_empty() {
            return Err(AppError::Conflict("inbound source id is empty".to_owned()));
        }
        Ok(Self {
            source_id,
            path: path.into(),
        })
    }
}

impl InboundSource for JsonLinesInbound {
    fn source_id(&self) -> &str {
        &self.source_id
    }

    fn poll(&mut self) -> Result<Vec<InboundRecord>, AppError> {
        reject_symlink(&self.path, "inbound file")?;
        let input = read_to_string(&self.path).map_err(AppError::worker)?;
        input
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let record: FileInboundRecord =
                    serde_json::from_str(line).map_err(AppError::outcome_schema)?;
                let signal = match record.signal.as_str() {
                    "pending" => InboundSignal::Pending,
                    "ready" => InboundSignal::Ready,
                    other => {
                        return Err(AppError::outcome_schema(format!(
                            "unknown inbound signal {other}"
                        )));
                    }
                };
                Ok(InboundRecord {
                    record_id: record.record_id,
                    signal,
                    project_id: ProjectId::parse(record.project_id)?,
                    repository: record.repository,
                    goal: record.goal,
                    worker_profile: record.worker_profile,
                    notification_target: record.notification_target,
                })
            })
            .collect()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PublishedRecord<'a> {
    schema_version: u32,
    publication_id: i64,
    kind: &'a str,
    work_id: &'a str,
    project_id: &'a str,
    target: &'a str,
    status: &'a str,
    created_at_unix_ms: u64,
}

/// Payload-minimal JSON-lines publisher. It never receives a goal or secret.
pub struct JsonLinesPublisher {
    path: PathBuf,
}

impl JsonLinesPublisher {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl Publisher for JsonLinesPublisher {
    fn publish(&mut self, publication: &Publication) -> Result<(), AppError> {
        reject_symlink(&self.path, "publisher output")?;
        let record = serde_json::to_string(&PublishedRecord {
            schema_version: 1,
            publication_id: publication.publication_id,
            kind: publication.kind.as_str(),
            work_id: publication.work_id.as_str(),
            project_id: publication.project_id.as_str(),
            target: &publication.target,
            status: publication.status.as_str(),
            created_at_unix_ms: publication.created_at_unix_ms,
        })
        .map_err(AppError::worker)?;
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        no_follow(&mut options);
        let mut file = options.open(&self.path).map_err(AppError::worker)?;
        writeln!(file, "{record}").map_err(AppError::worker)
    }
}

/// Run one source without coupling its latency to any other source or project.
pub fn spawn_inbound_once<S, T>(
    mut source: S,
    mut store: T,
) -> JoinHandle<Result<Vec<Work>, AppError>>
where
    S: InboundSource + Send + 'static,
    T: InboundStore + Send + 'static,
{
    thread::spawn(move || poll_inbound(&mut store, &mut source, &SystemClock))
}

/// Run one publisher independently. The caller gives each adapter its own store connection.
pub fn spawn_publisher_once<P, T>(
    mut publisher: P,
    mut store: T,
    limit: usize,
) -> JoinHandle<Result<usize, AppError>>
where
    P: Publisher + Send + 'static,
    T: PublicationStore + Send + 'static,
{
    thread::spawn(move || dispatch_publications(&mut store, &mut publisher, &SystemClock, limit))
}

fn reject_symlink(path: &Path, label: &str) -> Result<(), AppError> {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(AppError::worker(format!("{label} must not be a symlink")))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::worker(error)),
    }
}

fn no_follow(options: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;
    use workengine_adapters_store::SqliteStore;
    use workengine_application::{PublicationKind, WorkQuery, WorkStore};
    use workengine_domain::{WorkAttributes, WorkEvent, WorkId, WorkStatus};

    #[test]
    fn file_inbound_requires_typed_explicit_signal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("inbox.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"recordId\":\"a\",\"signal\":\"pending\",\"projectId\":\"p\",\"goal\":\"data\",\"workerProfile\":\"stub\"}\n",
                "{\"recordId\":\"b\",\"signal\":\"ready\",\"projectId\":\"p\",\"repository\":\"repo\",\"goal\":\"data\",\"workerProfile\":\"stub\",\"notificationTarget\":\"operator\"}\n"
            ),
        )
        .unwrap();
        let mut source = JsonLinesInbound::new("source", path).unwrap();
        let mut store = SqliteStore::open(dir.path().join("data")).unwrap();
        let created = poll_inbound(&mut store, &mut source, &SystemClock).unwrap();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].attributes().project_id().as_str(), "p");
        assert_eq!(
            poll_inbound(&mut store, &mut source, &SystemClock)
                .unwrap()
                .len(),
            0
        );
    }

    struct BlockingSource {
        id: &'static str,
        release: mpsc::Receiver<()>,
    }

    impl InboundSource for BlockingSource {
        fn source_id(&self) -> &str {
            self.id
        }

        fn poll(&mut self) -> Result<Vec<InboundRecord>, AppError> {
            self.release.recv().map_err(AppError::worker)?;
            Ok(Vec::new())
        }
    }

    struct EmptySource(&'static str);

    impl InboundSource for EmptySource {
        fn source_id(&self) -> &str {
            self.0
        }

        fn poll(&mut self) -> Result<Vec<InboundRecord>, AppError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn slow_source_does_not_block_an_unrelated_source() {
        let slow_dir = tempfile::tempdir().unwrap();
        let fast_dir = tempfile::tempdir().unwrap();
        let (release_tx, release_rx) = mpsc::channel();
        let slow = spawn_inbound_once(
            BlockingSource {
                id: "slow",
                release: release_rx,
            },
            SqliteStore::open(slow_dir.path()).unwrap(),
        );
        let fast = spawn_inbound_once(
            EmptySource("fast"),
            SqliteStore::open(fast_dir.path()).unwrap(),
        );
        assert!(fast.join().unwrap().unwrap().is_empty());
        release_tx.send(()).unwrap();
        assert!(slow.join().unwrap().unwrap().is_empty());
    }

    #[test]
    fn json_lines_publisher_contains_only_bounded_control_plane_fields() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("published.jsonl");
        let mut publisher = JsonLinesPublisher::new(&output);
        publisher
            .publish(&Publication {
                publication_id: 1,
                kind: PublicationKind::WorkTransition,
                work_id: WorkId::parse("work-a").unwrap(),
                project_id: ProjectId::parse("project-a").unwrap(),
                target: "operator-a".to_owned(),
                status: WorkStatus::Succeeded,
                created_at_unix_ms: 1,
                attempt_count: 0,
            })
            .unwrap();
        let text = std::fs::read_to_string(output).unwrap();
        assert!(text.contains("operator-a"));
        assert!(!text.contains("goal"));
        assert!(!text.contains("secret"));
    }

    #[test]
    fn publisher_job_uses_a_separate_store_connection_from_work_mutations() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = SqliteStore::open(dir.path()).unwrap();
        let work = Work::new(
            WorkId::parse("work-a").unwrap(),
            WorkAttributes::scoped(
                "goal",
                "stub",
                ProjectId::parse("project-a").unwrap(),
                None,
                Some("operator-a".to_owned()),
            )
            .unwrap(),
            1,
        )
        .unwrap();
        writer.put(&work, WorkEvent::created(&work)).unwrap();
        let output = dir.path().join("publish.jsonl");
        let job = spawn_publisher_once(
            JsonLinesPublisher::new(&output),
            SqliteStore::open(dir.path()).unwrap(),
            10,
        );
        let unrelated = Work::new(
            WorkId::parse("work-b").unwrap(),
            WorkAttributes::new("other", "stub").unwrap(),
            2,
        )
        .unwrap();
        writer
            .put(&unrelated, WorkEvent::created(&unrelated))
            .unwrap();
        assert_eq!(job.join().unwrap().unwrap(), 1);
        assert_eq!(writer.list().unwrap().len(), 2);
    }
}
