//! SQLite WorkStore. Status and event append are one transaction.

use std::path::Path;
use std::str::FromStr;

use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use workengine_application::{
    AppError, AttemptClaim, AttemptObservation, AttemptRecorder, AttemptState, CaptureLease,
    CaptureRequest, ConfirmedOutcomeObservation, ControlDirective, ControlKind,
    ExecutionObservation, ExecutionSpecObservation, InboundStore, OperatorInput, OperatorInputKind,
    ProcessEvent, ProcessRecordObservation, Publication, PublicationKind, PublicationStore,
    QueueStore, QuotaLease, QuotaStore, RelationStore, SecretRefObservation, SequencedEvent,
    WorkQuery, WorkStore,
};
use workengine_domain::{
    AttemptId, CONFIRMED_OUTCOME_SCHEMA_VERSION, ConfirmedOutcome, ContentDigest,
    EVENT_SCHEMA_VERSION, EXECUTION_SPEC_SCHEMA_VERSION, EventKind, ExecutionId, ExecutionSpec,
    OutcomeKind, ProjectId, RelationKind, SecretRef, SecretSource, Work, WorkAttributes, WorkEvent,
    WorkId, WorkRelation, WorkStatus,
};

const WORK_SCHEMA_VERSION: u32 = 1;
/// SQLite `user_version`. Distinct from per-row `schema_version` on Work.
const STORE_USER_VERSION: i32 = 6;

const MIGRATION_1: &str = "
CREATE TABLE IF NOT EXISTS works (
    id TEXT PRIMARY KEY,
    status TEXT NOT NULL,
    goal TEXT NOT NULL,
    worker_profile TEXT NOT NULL,
    workspace_root TEXT,
    created_at_unix_ms INTEGER NOT NULL,
    schema_version INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    work_id TEXT NOT NULL REFERENCES works(id),
    payload TEXT NOT NULL
);
";

const MIGRATION_2: &str = "
ALTER TABLE works ADD COLUMN generation INTEGER NOT NULL DEFAULT 0;
CREATE TABLE IF NOT EXISTS executions (
    id TEXT PRIMARY KEY,
    work_id TEXT NOT NULL REFERENCES works(id),
    spec_digest TEXT NOT NULL,
    created_at_unix_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS attempts (
    id TEXT PRIMARY KEY,
    execution_id TEXT NOT NULL REFERENCES executions(id),
    state TEXT NOT NULL,
    created_at_unix_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS control_requests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    work_id TEXT NOT NULL REFERENCES works(id),
    kind TEXT NOT NULL,
    state TEXT NOT NULL,
    created_at_unix_ms INTEGER NOT NULL
);
CREATE TRIGGER IF NOT EXISTS events_are_append_only_update
BEFORE UPDATE ON events BEGIN SELECT RAISE(ABORT, 'events are append-only'); END;
CREATE TRIGGER IF NOT EXISTS events_are_append_only_delete
BEFORE DELETE ON events BEGIN SELECT RAISE(ABORT, 'events are append-only'); END;
";

const MIGRATION_3: &str = "
ALTER TABLE works ADD COLUMN active_execution_id TEXT;
ALTER TABLE works ADD COLUMN active_attempt_id TEXT;
ALTER TABLE executions ADD COLUMN spec_payload TEXT NOT NULL DEFAULT '{}';
ALTER TABLE attempts ADD COLUMN terminal_reason TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS one_active_attempt_per_execution
    ON attempts(execution_id) WHERE state = 'active';
CREATE TABLE IF NOT EXISTS confirmed_outcomes (
    attempt_id TEXT PRIMARY KEY REFERENCES attempts(id),
    execution_id TEXT NOT NULL REFERENCES executions(id),
    work_id TEXT NOT NULL REFERENCES works(id),
    payload TEXT NOT NULL,
    confirmed_at_unix_ms INTEGER NOT NULL
);
";

const MIGRATION_4: &str = "
ALTER TABLE attempts ADD COLUMN last_heartbeat_at_unix_ms INTEGER;
ALTER TABLE attempts ADD COLUMN finished_at_unix_ms INTEGER;
UPDATE attempts
SET last_heartbeat_at_unix_ms = created_at_unix_ms
WHERE last_heartbeat_at_unix_ms IS NULL;
CREATE TABLE IF NOT EXISTS attempt_process_records (
    attempt_id TEXT NOT NULL REFERENCES attempts(id),
    execution_id TEXT NOT NULL REFERENCES executions(id),
    work_id TEXT NOT NULL REFERENCES works(id),
    event TEXT NOT NULL,
    occurrences INTEGER NOT NULL,
    first_observed_at_unix_ms INTEGER NOT NULL,
    last_observed_at_unix_ms INTEGER NOT NULL,
    payload_redacted INTEGER NOT NULL,
    PRIMARY KEY (attempt_id, event)
);
";

const MIGRATION_5: &str = "
ALTER TABLE attempts ADD COLUMN checkpoint_recorded INTEGER NOT NULL DEFAULT 0;
ALTER TABLE control_requests ADD COLUMN execution_id TEXT;
ALTER TABLE control_requests ADD COLUMN attempt_id TEXT;
ALTER TABLE control_requests ADD COLUMN finished_at_unix_ms INTEGER;
CREATE UNIQUE INDEX IF NOT EXISTS one_pending_control_per_work
    ON control_requests(work_id) WHERE state = 'pending';
CREATE TABLE IF NOT EXISTS operator_inputs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    work_id TEXT NOT NULL REFERENCES works(id),
    kind TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at_unix_ms INTEGER NOT NULL
);
";

const MIGRATION_6: &str = "
ALTER TABLE works ADD COLUMN project_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE works ADD COLUMN repository TEXT;
ALTER TABLE works ADD COLUMN notification_target TEXT;
CREATE INDEX IF NOT EXISTS works_project_queue
    ON works(project_id, status, created_at_unix_ms);
CREATE TABLE IF NOT EXISTS work_relations (
    from_work_id TEXT NOT NULL REFERENCES works(id),
    to_work_id TEXT NOT NULL REFERENCES works(id),
    kind TEXT NOT NULL,
    PRIMARY KEY (from_work_id, to_work_id, kind),
    CHECK (from_work_id <> to_work_id)
);
CREATE TABLE IF NOT EXISTS captures (
    id TEXT PRIMARY KEY,
    work_id TEXT NOT NULL REFERENCES works(id),
    project_id TEXT NOT NULL,
    worker_id TEXT NOT NULL,
    work_generation INTEGER NOT NULL,
    state TEXT NOT NULL,
    captured_at_unix_ms INTEGER NOT NULL,
    finished_at_unix_ms INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS one_active_capture_per_work
    ON captures(work_id) WHERE state = 'active';
CREATE TABLE IF NOT EXISTS quota_limits (
    resource TEXT PRIMARY KEY,
    unit_limit INTEGER NOT NULL,
    generation INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS quota_leases (
    id TEXT PRIMARY KEY,
    resource TEXT NOT NULL REFERENCES quota_limits(resource),
    holder TEXT NOT NULL,
    units INTEGER NOT NULL,
    state TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS active_quota_usage
    ON quota_leases(resource, state);
CREATE TABLE IF NOT EXISTS inbound_receipts (
    source_id TEXT NOT NULL,
    record_id TEXT NOT NULL,
    work_id TEXT NOT NULL UNIQUE REFERENCES works(id),
    PRIMARY KEY (source_id, record_id)
);
CREATE TABLE IF NOT EXISTS publications (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    work_id TEXT NOT NULL REFERENCES works(id),
    project_id TEXT NOT NULL,
    target TEXT NOT NULL,
    status TEXT NOT NULL,
    state TEXT NOT NULL,
    created_at_unix_ms INTEGER NOT NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    last_attempt_at_unix_ms INTEGER,
    last_error TEXT
);
CREATE INDEX IF NOT EXISTS pending_publications
    ON publications(state, id);
";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EventPayload {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    #[serde(rename = "workId")]
    work_id: String,
    kind: String,
    from: Option<String>,
    to: String,
    goal: Option<String>,
    #[serde(rename = "workerProfile")]
    worker_profile: Option<String>,
    #[serde(rename = "projectId", default)]
    project_id: Option<String>,
    #[serde(default)]
    repository: Option<String>,
    #[serde(rename = "notificationTarget", default)]
    notification_target: Option<String>,
    #[serde(rename = "outcomeKind")]
    outcome_kind: Option<String>,
    #[serde(rename = "workspaceRoot")]
    workspace_root: Option<String>,
    #[serde(rename = "createdAtUnixMs")]
    created_at_unix_ms: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExecutionSpecPayload<'a> {
    schema_version: u32,
    work_id: &'a str,
    worker_profile: &'a str,
    worker_config_digest: &'a str,
    runtime_kind: &'a str,
    runtime_digest: &'a str,
    wall_clock_budget_ms: u64,
    retry_limit: u32,
    channel_policy: &'a str,
    secret_refs: Vec<SecretRefPayload<'a>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredExecutionSpecPayload {
    schema_version: u32,
    work_id: String,
    worker_profile: String,
    worker_config_digest: String,
    #[serde(default)]
    runtime_kind: Option<String>,
    runtime_digest: String,
    wall_clock_budget_ms: u64,
    retry_limit: u32,
    channel_policy: String,
    secret_refs: Vec<StoredSecretRefPayload>,
}

#[derive(Deserialize)]
struct StoredSecretRefPayload {
    name: String,
    source: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SecretRefPayload<'a> {
    name: &'a str,
    source: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfirmedOutcomePayload<'a> {
    schema_version: u32,
    work_id: &'a str,
    execution_id: &'a str,
    attempt_id: &'a str,
    kind: &'a str,
    worker_profile: &'a str,
    confirmed_at_unix_ms: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredConfirmedOutcomePayload {
    schema_version: u32,
    work_id: String,
    execution_id: String,
    attempt_id: String,
    kind: String,
    worker_profile: String,
    confirmed_at_unix_ms: u64,
}

pub struct SqliteStore {
    conn: Connection,
}

/// A read-only SQLite connection for HTTP and CLI observers.
///
/// It opens the database with SQLite's read-only flag and never runs recovery
/// or schema migrations.
pub struct SqliteObserver {
    conn: Connection,
}

impl SqliteStore {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, AppError> {
        let data_dir = data_dir.as_ref();
        reject_symlink(data_dir, "data directory")?;
        std::fs::create_dir_all(data_dir).map_err(AppError::store)?;
        restrict_directory(data_dir)?;
        let database = data_dir.join("workengine.sqlite");
        reject_symlink(&database, "store database")?;
        reject_sqlite_sidecar_symlinks(&database)?;
        let conn = Connection::open(&database).map_err(AppError::store)?;
        restrict_file(&database)?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
            .map_err(AppError::store)?;
        migrate(&conn)?;
        Ok(Self { conn })
    }

    fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkRow> {
        Ok(WorkRow {
            id: row.get(0)?,
            status: row.get(1)?,
            goal: row.get(2)?,
            worker_profile: row.get(3)?,
            workspace_root: row.get(4)?,
            created_at_unix_ms: row.get(5)?,
            schema_version: row.get(6)?,
            project_id: row.get(7)?,
            repository: row.get(8)?,
            notification_target: row.get(9)?,
        })
    }

    fn work_from_row(row: WorkRow) -> Result<Work, AppError> {
        if row.schema_version as u32 != WORK_SCHEMA_VERSION {
            return Err(AppError::from(
                workengine_domain::DomainError::UnsupportedSchemaVersion(row.schema_version as u32),
            ));
        }
        Ok(Work::restore(
            WorkId::parse(row.id)?,
            WorkStatus::from_str(&row.status)?,
            WorkAttributes::scoped(
                row.goal,
                row.worker_profile,
                ProjectId::parse(row.project_id)?,
                row.repository,
                row.notification_target,
            )?,
            row.workspace_root,
            row.created_at_unix_ms as u64,
        ))
    }
}

impl SqliteObserver {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, AppError> {
        let database = data_dir.as_ref().join("workengine.sqlite");
        reject_symlink(&database, "store database")?;
        reject_sqlite_sidecar_symlinks(&database)?;
        let conn = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(AppError::store)?;
        conn.execute_batch("PRAGMA query_only = ON; PRAGMA foreign_keys = ON;")
            .map_err(AppError::store)?;
        Ok(Self { conn })
    }
}

fn reject_symlink(path: &Path, label: &str) -> Result<(), AppError> {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(AppError::store(format!("{label} must not be a symlink")))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::store(error)),
    }
}

fn reject_sqlite_sidecar_symlinks(database: &Path) -> Result<(), AppError> {
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = database.as_os_str().to_owned();
        sidecar.push(suffix);
        reject_symlink(Path::new(&sidecar), "store sidecar")?;
    }
    Ok(())
}

fn events_after_connection(
    conn: &Connection,
    after_seq: u64,
    work_id: Option<&WorkId>,
) -> Result<Vec<SequencedEvent>, AppError> {
    let sql = if work_id.is_some() {
        "SELECT seq, payload FROM events WHERE seq > ?1 AND work_id = ?2 ORDER BY seq"
    } else {
        "SELECT seq, payload FROM events WHERE seq > ?1 ORDER BY seq"
    };
    let mut statement = conn.prepare(sql).map_err(AppError::store)?;
    let mut rows = if let Some(id) = work_id {
        statement
            .query(params![after_seq as i64, id.as_str()])
            .map_err(AppError::store)?
    } else {
        statement
            .query(params![after_seq as i64])
            .map_err(AppError::store)?
    };
    let mut records = Vec::new();
    while let Some(row) = rows.next().map_err(AppError::store)? {
        let seq: i64 = row.get(0).map_err(AppError::store)?;
        let payload: String = row.get(1).map_err(AppError::store)?;
        records.push(SequencedEvent {
            seq: seq as u64,
            event: decode_event(&payload)?,
        });
    }
    Ok(records)
}

struct WorkRow {
    id: String,
    status: String,
    goal: String,
    worker_profile: String,
    workspace_root: Option<String>,
    created_at_unix_ms: i64,
    schema_version: i64,
    project_id: String,
    repository: Option<String>,
    notification_target: Option<String>,
}

fn get_connection(conn: &Connection, id: &WorkId) -> Result<Option<Work>, AppError> {
    conn
        .query_row(
            "SELECT id, status, goal, worker_profile, workspace_root, created_at_unix_ms, schema_version,
                    project_id, repository, notification_target
                 FROM works WHERE id = ?1",
            [id.as_str()],
            SqliteStore::read_row,
        )
        .optional()
        .map_err(AppError::store)?
        .map(SqliteStore::work_from_row)
        .transpose()
}

fn list_connection(conn: &Connection) -> Result<Vec<Work>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, status, goal, worker_profile, workspace_root, created_at_unix_ms, schema_version,
                    project_id, repository, notification_target
                 FROM works",
        )
        .map_err(AppError::store)?;
    let rows = stmt
        .query_map([], SqliteStore::read_row)
        .map_err(AppError::store)?;
    let mut works = Vec::new();
    for row in rows {
        works.push(SqliteStore::work_from_row(row.map_err(AppError::store)?)?);
    }
    Ok(works)
}

fn events_connection(conn: &Connection, id: &WorkId) -> Result<Vec<WorkEvent>, AppError> {
    let mut stmt = conn
        .prepare("SELECT payload FROM events WHERE work_id = ?1 ORDER BY seq")
        .map_err(AppError::store)?;
    let rows = stmt
        .query_map([id.as_str()], |row| row.get::<_, String>(0))
        .map_err(AppError::store)?;
    let mut events = Vec::new();
    for row in rows {
        events.push(decode_event(&row.map_err(AppError::store)?)?);
    }
    Ok(events)
}

fn executions_connection(
    conn: &Connection,
    work_id: &WorkId,
) -> Result<Vec<ExecutionObservation>, AppError> {
    let mut statement = conn
        .prepare(
            "SELECT id, spec_payload, created_at_unix_ms
             FROM executions WHERE work_id = ?1
             ORDER BY created_at_unix_ms, id",
        )
        .map_err(AppError::store)?;
    let rows = statement
        .query_map([work_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(AppError::store)?;
    let mut executions = Vec::new();
    for row in rows {
        let (execution_id, payload, created_at) = row.map_err(AppError::store)?;
        let execution_id = ExecutionId::parse(execution_id)?;
        let payload: StoredExecutionSpecPayload =
            serde_json::from_str(&payload).map_err(AppError::outcome_schema)?;
        validate_execution_spec_payload(&payload)?;
        if payload.work_id != work_id.as_str() {
            return Err(AppError::store("execution payload has foreign Work id"));
        }
        let attempts = attempt_observations(conn, work_id, &execution_id)?;
        executions.push(ExecutionObservation {
            execution_id,
            work_id: work_id.clone(),
            created_at_unix_ms: unsigned(created_at, "execution created time")?,
            spec: ExecutionSpecObservation {
                schema_version: payload.schema_version,
                worker_profile: payload.worker_profile,
                worker_config_digest: payload.worker_config_digest,
                runtime_kind: payload.runtime_kind,
                runtime_digest: payload.runtime_digest,
                wall_clock_budget_ms: payload.wall_clock_budget_ms,
                retry_limit: payload.retry_limit,
                channel_policy: payload.channel_policy,
                secret_refs: payload
                    .secret_refs
                    .into_iter()
                    .map(|reference| SecretRefObservation {
                        name: reference.name,
                        source: reference.source,
                    })
                    .collect(),
            },
            attempts,
        });
    }
    Ok(executions)
}

fn attempt_observations(
    conn: &Connection,
    work_id: &WorkId,
    execution_id: &ExecutionId,
) -> Result<Vec<AttemptObservation>, AppError> {
    let mut statement = conn
        .prepare(
            "SELECT id, state, created_at_unix_ms,
                    COALESCE(last_heartbeat_at_unix_ms, created_at_unix_ms),
                    finished_at_unix_ms, terminal_reason, checkpoint_recorded
             FROM attempts WHERE execution_id = ?1
             ORDER BY created_at_unix_ms, id",
        )
        .map_err(AppError::store)?;
    let rows = statement
        .query_map([execution_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, bool>(6)?,
            ))
        })
        .map_err(AppError::store)?;
    let mut attempts = Vec::new();
    for (retry_ordinal, row) in rows.enumerate() {
        let (
            attempt_id,
            state,
            started_at,
            heartbeat_at,
            finished_at,
            terminal_reason,
            checkpoint_recorded,
        ) = row.map_err(AppError::store)?;
        let attempt_id = AttemptId::parse(attempt_id)?;
        attempts.push(AttemptObservation {
            process_records: process_record_observations(conn, &attempt_id)?,
            confirmed_outcome: confirmed_outcome_observation(
                conn,
                work_id,
                execution_id,
                &attempt_id,
            )?,
            attempt_id,
            state: AttemptState::parse(&state)?,
            retry_ordinal: u32::try_from(retry_ordinal)
                .map_err(|_| AppError::store("attempt retry ordinal overflow"))?,
            started_at_unix_ms: unsigned(started_at, "attempt start time")?,
            last_heartbeat_at_unix_ms: unsigned(heartbeat_at, "attempt heartbeat time")?,
            finished_at_unix_ms: finished_at
                .map(|value| unsigned(value, "attempt finish time"))
                .transpose()?,
            terminal_reason,
            checkpoint_recorded,
        });
    }
    Ok(attempts)
}

fn process_record_observations(
    conn: &Connection,
    attempt_id: &AttemptId,
) -> Result<Vec<ProcessRecordObservation>, AppError> {
    let mut statement = conn
        .prepare(
            "SELECT event, occurrences, first_observed_at_unix_ms,
                    last_observed_at_unix_ms, payload_redacted
             FROM attempt_process_records WHERE attempt_id = ?1
             ORDER BY first_observed_at_unix_ms, event",
        )
        .map_err(AppError::store)?;
    let rows = statement
        .query_map([attempt_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, bool>(4)?,
            ))
        })
        .map_err(AppError::store)?;
    let mut records = Vec::new();
    for row in rows {
        let (event, occurrences, first_at, last_at, payload_redacted) =
            row.map_err(AppError::store)?;
        records.push(ProcessRecordObservation {
            event: ProcessEvent::parse(&event)?,
            occurrences: unsigned(occurrences, "process record occurrences")?,
            first_observed_at_unix_ms: unsigned(first_at, "process record first time")?,
            last_observed_at_unix_ms: unsigned(last_at, "process record last time")?,
            payload_redacted,
        });
    }
    Ok(records)
}

fn confirmed_outcome_observation(
    conn: &Connection,
    work_id: &WorkId,
    execution_id: &ExecutionId,
    attempt_id: &AttemptId,
) -> Result<Option<ConfirmedOutcomeObservation>, AppError> {
    let payload: Option<String> = conn
        .query_row(
            "SELECT payload FROM confirmed_outcomes
             WHERE work_id = ?1 AND execution_id = ?2 AND attempt_id = ?3",
            params![work_id.as_str(), execution_id.as_str(), attempt_id.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(AppError::store)?;
    let Some(payload) = payload else {
        return Ok(None);
    };
    let payload: StoredConfirmedOutcomePayload =
        serde_json::from_str(&payload).map_err(AppError::outcome_schema)?;
    if payload.schema_version != CONFIRMED_OUTCOME_SCHEMA_VERSION {
        return Err(AppError::from(
            workengine_domain::DomainError::UnsupportedSchemaVersion(payload.schema_version),
        ));
    }
    if payload.work_id != work_id.as_str()
        || payload.execution_id != execution_id.as_str()
        || payload.attempt_id != attempt_id.as_str()
    {
        return Err(AppError::store(
            "confirmed outcome payload has foreign provenance",
        ));
    }
    Ok(Some(ConfirmedOutcomeObservation {
        schema_version: payload.schema_version,
        kind: OutcomeKind::from_str(&payload.kind)?,
        worker_profile: payload.worker_profile,
        confirmed_at_unix_ms: payload.confirmed_at_unix_ms,
    }))
}

fn validate_execution_spec_payload(payload: &StoredExecutionSpecPayload) -> Result<(), AppError> {
    if payload.schema_version != EXECUTION_SPEC_SCHEMA_VERSION {
        return Err(AppError::from(
            workengine_domain::DomainError::UnsupportedSchemaVersion(payload.schema_version),
        ));
    }
    ContentDigest::parse(&payload.worker_config_digest)?;
    ContentDigest::parse(&payload.runtime_digest)?;
    if payload
        .runtime_kind
        .as_deref()
        .is_some_and(|kind| !matches!(kind, "stub" | "bubblewrap" | "oci"))
    {
        return Err(AppError::store(
            "execution payload has unknown runtime kind",
        ));
    }
    if !matches!(
        payload.channel_policy.as_str(),
        "fail" | "park" | "retry_then_fail"
    ) {
        return Err(AppError::store(
            "execution payload has unknown channel policy",
        ));
    }
    for reference in &payload.secret_refs {
        let source = SecretSource::environment_variable(&reference.source)?;
        SecretRef::new(&reference.name, source)?;
    }
    Ok(())
}

fn unsigned(value: i64, field: &str) -> Result<u64, AppError> {
    u64::try_from(value).map_err(|_| AppError::store(format!("negative {field}")))
}

macro_rules! impl_work_query {
    ($type:ty) => {
        impl WorkQuery for $type {
            fn get(&self, id: &WorkId) -> Result<Option<Work>, AppError> {
                get_connection(&self.conn, id)
            }

            fn list(&self) -> Result<Vec<Work>, AppError> {
                list_connection(&self.conn)
            }

            fn events(&self, id: &WorkId) -> Result<Vec<WorkEvent>, AppError> {
                events_connection(&self.conn, id)
            }

            fn events_after(
                &self,
                after_seq: u64,
                work_id: Option<&WorkId>,
            ) -> Result<Vec<SequencedEvent>, AppError> {
                events_after_connection(&self.conn, after_seq, work_id)
            }

            fn executions(&self, id: &WorkId) -> Result<Vec<ExecutionObservation>, AppError> {
                executions_connection(&self.conn, id)
            }
        }
    };
}

impl_work_query!(SqliteStore);
impl_work_query!(SqliteObserver);

impl AttemptRecorder for SqliteStore {
    fn heartbeat_attempt(
        &mut self,
        work_id: &WorkId,
        execution_id: &ExecutionId,
        attempt_id: &AttemptId,
        observed_at_unix_ms: u64,
    ) -> Result<(), AppError> {
        let changed = self
            .conn
            .execute(
                "UPDATE attempts
                 SET last_heartbeat_at_unix_ms = MAX(
                    COALESCE(last_heartbeat_at_unix_ms, created_at_unix_ms), ?1
                 )
                 WHERE id = ?2 AND execution_id = ?3 AND state = 'active'
                   AND EXISTS (
                     SELECT 1 FROM works
                     WHERE id = ?4 AND status = 'running'
                       AND active_execution_id = ?3 AND active_attempt_id = ?2
                   )",
                params![
                    observed_at_unix_ms as i64,
                    attempt_id.as_str(),
                    execution_id.as_str(),
                    work_id.as_str(),
                ],
            )
            .map_err(AppError::store)?;
        if changed != 1 {
            return Err(attempt_conflict(attempt_id));
        }
        Ok(())
    }

    fn record_process_event(
        &mut self,
        work_id: &WorkId,
        execution_id: &ExecutionId,
        attempt_id: &AttemptId,
        event: ProcessEvent,
        observed_at_unix_ms: u64,
    ) -> Result<(), AppError> {
        let tx = self.conn.transaction().map_err(AppError::store)?;
        let changed = tx
            .execute(
                "UPDATE attempts
                 SET last_heartbeat_at_unix_ms = MAX(
                    COALESCE(last_heartbeat_at_unix_ms, created_at_unix_ms), ?1
                 )
                 WHERE id = ?2 AND execution_id = ?3 AND state = 'active'
                   AND EXISTS (
                     SELECT 1 FROM works
                     WHERE id = ?4 AND status = 'running'
                       AND active_execution_id = ?3 AND active_attempt_id = ?2
                   )",
                params![
                    observed_at_unix_ms as i64,
                    attempt_id.as_str(),
                    execution_id.as_str(),
                    work_id.as_str(),
                ],
            )
            .map_err(AppError::store)?;
        if changed != 1 {
            return Err(attempt_conflict(attempt_id));
        }
        let payload_redacted =
            matches!(event, ProcessEvent::ChildStdout | ProcessEvent::ChildStderr);
        tx.execute(
            "INSERT INTO attempt_process_records
                 (attempt_id, execution_id, work_id, event, occurrences,
                  first_observed_at_unix_ms, last_observed_at_unix_ms, payload_redacted)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5, ?6)
             ON CONFLICT(attempt_id, event) DO UPDATE SET
                 occurrences = occurrences + 1,
                 last_observed_at_unix_ms = MAX(last_observed_at_unix_ms, excluded.last_observed_at_unix_ms),
                 payload_redacted = MAX(payload_redacted, excluded.payload_redacted)",
            params![
                attempt_id.as_str(),
                execution_id.as_str(),
                work_id.as_str(),
                event.as_str(),
                observed_at_unix_ms as i64,
                payload_redacted,
            ],
        )
        .map_err(AppError::store)?;
        tx.commit().map_err(AppError::store)
    }

    fn control_directive(
        &mut self,
        work_id: &WorkId,
        execution_id: &ExecutionId,
        attempt_id: &AttemptId,
    ) -> Result<Option<ControlDirective>, AppError> {
        self.conn
            .query_row(
                "SELECT id, kind FROM control_requests
                 WHERE work_id = ?1 AND execution_id = ?2 AND attempt_id = ?3
                   AND state = 'pending'
                 ORDER BY id LIMIT 1",
                params![work_id.as_str(), execution_id.as_str(), attempt_id.as_str()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(AppError::store)?
            .map(|(request_id, kind)| {
                Ok(ControlDirective {
                    request_id,
                    kind: ControlKind::parse(&kind)?,
                })
            })
            .transpose()
    }

    fn record_checkpoint(
        &mut self,
        work_id: &WorkId,
        execution_id: &ExecutionId,
        attempt_id: &AttemptId,
        observed_at_unix_ms: u64,
    ) -> Result<(), AppError> {
        let changed = self
            .conn
            .execute(
                "UPDATE attempts
                 SET checkpoint_recorded = 1,
                     last_heartbeat_at_unix_ms = MAX(
                       COALESCE(last_heartbeat_at_unix_ms, created_at_unix_ms), ?1
                     )
                 WHERE id = ?2 AND execution_id = ?3 AND state = 'active'
                   AND EXISTS (
                     SELECT 1 FROM works WHERE id = ?4 AND status = 'running'
                       AND active_execution_id = ?3 AND active_attempt_id = ?2
                   )",
                params![
                    observed_at_unix_ms as i64,
                    attempt_id.as_str(),
                    execution_id.as_str(),
                    work_id.as_str(),
                ],
            )
            .map_err(AppError::store)?;
        if changed != 1 {
            return Err(attempt_conflict(attempt_id));
        }
        Ok(())
    }
}

impl WorkStore for SqliteStore {
    fn put(&mut self, work: &Work, event: WorkEvent) -> Result<(), AppError> {
        let tx = self.conn.transaction().map_err(AppError::store)?;
        put_in_tx(&tx, work, &event)?;
        tx.commit().map_err(AppError::store)?;
        Ok(())
    }

    fn claim_attempt(&mut self, claim: &AttemptClaim<'_>) -> Result<(), AppError> {
        let tx = self.conn.transaction().map_err(AppError::store)?;
        if let Some(capture) = claim.capture {
            if capture.work.id() != claim.work.id()
                || capture.work.attributes().project_id() != claim.work.attributes().project_id()
            {
                return Err(AppError::Conflict(
                    "capture does not belong to the attempted Work".to_owned(),
                ));
            }
            let consumed = tx
                .execute(
                    "UPDATE captures SET state = 'consumed', finished_at_unix_ms = ?1
                     WHERE id = ?2 AND work_id = ?3 AND worker_id = ?4
                       AND work_generation = ?5 AND state = 'active'
                       AND EXISTS (
                         SELECT 1 FROM works WHERE id = ?3 AND generation = ?5
                       )",
                    params![
                        claim.started_at_unix_ms as i64,
                        capture.capture_id,
                        claim.work.id().as_str(),
                        capture.worker_id,
                        capture.generation as i64,
                    ],
                )
                .map_err(AppError::store)?;
            if consumed != 1 {
                return Err(AppError::Conflict("capture lease was lost".to_owned()));
            }
        } else {
            let captured: bool = tx
                .query_row(
                    "SELECT EXISTS(
                       SELECT 1 FROM captures WHERE work_id = ?1 AND state = 'active'
                     )",
                    [claim.work.id().as_str()],
                    |row| row.get(0),
                )
                .map_err(AppError::store)?;
            if captured {
                return Err(AppError::Conflict(format!(
                    "Work {} has an active queue capture",
                    claim.work.id()
                )));
            }
            acquire_queue_quotas(&tx, claim.work)?;
        }
        let spec_payload = encode_execution_spec(claim.spec)?;
        let existing: Option<(String, String)> = tx
            .query_row(
                "SELECT work_id, spec_payload FROM executions WHERE id = ?1",
                [claim.execution_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(AppError::store)?;
        if let Some((work_id, payload)) = existing {
            if work_id != claim.work.id().as_str() || payload != spec_payload {
                return Err(AppError::Conflict(
                    "resume configuration differs from the immutable execution spec".to_owned(),
                ));
            }
        } else {
            tx.execute(
                "INSERT INTO executions (id, work_id, spec_digest, created_at_unix_ms, spec_payload)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    claim.execution_id.as_str(),
                    claim.work.id().as_str(),
                    claim.spec.worker_config_digest().as_str(),
                    claim.started_at_unix_ms as i64,
                    spec_payload,
                ],
            )
            .map_err(map_constraint_conflict)?;
        }
        tx.execute(
            "INSERT INTO attempts
                 (id, execution_id, state, created_at_unix_ms, last_heartbeat_at_unix_ms)
             VALUES (?1, ?2, 'active', ?3, ?3)",
            params![
                claim.attempt_id.as_str(),
                claim.execution_id.as_str(),
                claim.started_at_unix_ms as i64,
            ],
        )
        .map_err(map_constraint_conflict)?;
        let from = claim
            .event
            .from()
            .ok_or_else(|| AppError::store("start event has no prior status"))?;
        let changed = tx
            .execute(
                "UPDATE works
                 SET status = ?1, workspace_root = ?2, generation = generation + 1,
                     active_execution_id = ?3, active_attempt_id = ?4
                 WHERE id = ?5 AND status = ?6 AND active_attempt_id IS NULL",
                params![
                    claim.work.status().as_str(),
                    claim.work.workspace_root(),
                    claim.execution_id.as_str(),
                    claim.attempt_id.as_str(),
                    claim.work.id().as_str(),
                    from.as_str(),
                ],
            )
            .map_err(AppError::store)?;
        if changed != 1 {
            return Err(AppError::Conflict(format!(
                "Work {} already has an active attempt or changed status",
                claim.work.id()
            )));
        }
        insert_event(&tx, &claim.event)?;
        tx.commit().map_err(AppError::store)
    }

    fn retry_attempt(
        &mut self,
        execution_id: &ExecutionId,
        previous_attempt_id: &AttemptId,
        next_attempt_id: &AttemptId,
        started_at_unix_ms: u64,
    ) -> Result<(), AppError> {
        let tx = self.conn.transaction().map_err(AppError::store)?;
        let changed = tx
            .execute(
                "UPDATE attempts
                 SET state = 'retried', terminal_reason = 'channel_retry',
                     finished_at_unix_ms = ?3
                 WHERE id = ?1 AND execution_id = ?2 AND state = 'active'",
                params![
                    previous_attempt_id.as_str(),
                    execution_id.as_str(),
                    started_at_unix_ms as i64,
                ],
            )
            .map_err(AppError::store)?;
        if changed != 1 {
            return Err(attempt_conflict(previous_attempt_id));
        }
        tx.execute(
            "INSERT INTO attempts
                 (id, execution_id, state, created_at_unix_ms, last_heartbeat_at_unix_ms)
             VALUES (?1, ?2, 'active', ?3, ?3)",
            params![
                next_attempt_id.as_str(),
                execution_id.as_str(),
                started_at_unix_ms as i64,
            ],
        )
        .map_err(map_constraint_conflict)?;
        let changed = tx
            .execute(
                "UPDATE works SET active_attempt_id = ?1, generation = generation + 1
                 WHERE active_execution_id = ?2 AND active_attempt_id = ?3 AND status = 'running'",
                params![
                    next_attempt_id.as_str(),
                    execution_id.as_str(),
                    previous_attempt_id.as_str(),
                ],
            )
            .map_err(AppError::store)?;
        if changed != 1 {
            return Err(attempt_conflict(previous_attempt_id));
        }
        tx.commit().map_err(AppError::store)
    }

    fn confirm_attempt(
        &mut self,
        work: &Work,
        event: WorkEvent,
        outcome: &ConfirmedOutcome,
    ) -> Result<(), AppError> {
        let reason = outcome.outcome().kind().as_str();
        self.confirm_attempt_with_reason(work, event, outcome, reason, None)
    }

    fn confirm_attempt_with_reason(
        &mut self,
        work: &Work,
        event: WorkEvent,
        outcome: &ConfirmedOutcome,
        terminal_reason: &str,
        control_request_id: Option<i64>,
    ) -> Result<(), AppError> {
        let tx = self.conn.transaction().map_err(AppError::store)?;
        let changed = tx
            .execute(
                "UPDATE works
                 SET status = ?1, workspace_root = ?2, generation = generation + 1,
                     active_execution_id = NULL, active_attempt_id = NULL
                 WHERE id = ?3 AND status = 'running'
                   AND active_execution_id = ?4 AND active_attempt_id = ?5",
                params![
                    work.status().as_str(),
                    work.workspace_root(),
                    work.id().as_str(),
                    outcome.execution_id().as_str(),
                    outcome.attempt_id().as_str(),
                ],
            )
            .map_err(AppError::store)?;
        if changed != 1 {
            return Err(attempt_conflict(outcome.attempt_id()));
        }
        release_queue_quotas(&tx, work)?;
        let changed = tx
            .execute(
                "UPDATE attempts
                 SET state = 'confirmed', terminal_reason = ?1,
                     finished_at_unix_ms = ?4,
                     last_heartbeat_at_unix_ms = MAX(
                         COALESCE(last_heartbeat_at_unix_ms, created_at_unix_ms), ?4
                     )
                 WHERE id = ?2 AND execution_id = ?3 AND state = 'active'",
                params![
                    terminal_reason,
                    outcome.attempt_id().as_str(),
                    outcome.execution_id().as_str(),
                    outcome.confirmed_at_unix_ms() as i64,
                ],
            )
            .map_err(AppError::store)?;
        if changed != 1 {
            return Err(attempt_conflict(outcome.attempt_id()));
        }
        tx.execute(
            "INSERT INTO confirmed_outcomes
                 (attempt_id, execution_id, work_id, payload, confirmed_at_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                outcome.attempt_id().as_str(),
                outcome.execution_id().as_str(),
                outcome.work_id().as_str(),
                encode_confirmed_outcome(outcome)?,
                outcome.confirmed_at_unix_ms() as i64,
            ],
        )
        .map_err(map_constraint_conflict)?;
        resolve_controls(
            &tx,
            outcome.work_id(),
            control_request_id,
            outcome.confirmed_at_unix_ms(),
        )?;
        insert_event(&tx, &event)?;
        tx.commit().map_err(AppError::store)
    }

    fn park_attempt(
        &mut self,
        work: &Work,
        event: WorkEvent,
        execution_id: &ExecutionId,
        attempt_id: &AttemptId,
    ) -> Result<(), AppError> {
        self.park_attempt_with_checkpoint(work, event, execution_id, attempt_id, true, None)
    }

    fn park_attempt_with_checkpoint(
        &mut self,
        work: &Work,
        event: WorkEvent,
        execution_id: &ExecutionId,
        attempt_id: &AttemptId,
        checkpoint_recorded: bool,
        control_request_id: Option<i64>,
    ) -> Result<(), AppError> {
        if !checkpoint_recorded {
            return Err(AppError::Conflict(
                "park requires a validated attempt checkpoint".to_owned(),
            ));
        }
        let tx = self.conn.transaction().map_err(AppError::store)?;
        let changed = tx
            .execute(
                "UPDATE works
                 SET status = 'parked', workspace_root = ?1, generation = generation + 1,
                     active_execution_id = NULL, active_attempt_id = NULL
                 WHERE id = ?2 AND status = 'running'
                   AND active_execution_id = ?3 AND active_attempt_id = ?4",
                params![
                    work.workspace_root(),
                    work.id().as_str(),
                    execution_id.as_str(),
                    attempt_id.as_str(),
                ],
            )
            .map_err(AppError::store)?;
        if changed != 1 {
            return Err(attempt_conflict(attempt_id));
        }
        release_queue_quotas(&tx, work)?;
        let changed = tx
            .execute(
                "UPDATE attempts
                 SET state = 'parked', terminal_reason = 'parked',
                     checkpoint_recorded = 1,
                     finished_at_unix_ms = ?3,
                     last_heartbeat_at_unix_ms = MAX(
                         COALESCE(last_heartbeat_at_unix_ms, created_at_unix_ms), ?3
                     )
                 WHERE id = ?1 AND execution_id = ?2 AND state = 'active'",
                params![
                    attempt_id.as_str(),
                    execution_id.as_str(),
                    event.created_at_unix_ms() as i64,
                ],
            )
            .map_err(AppError::store)?;
        if changed != 1 {
            return Err(attempt_conflict(attempt_id));
        }
        resolve_controls(
            &tx,
            work.id(),
            control_request_id,
            event.created_at_unix_ms(),
        )?;
        if control_request_id.is_none() {
            enqueue_publication(&tx, &event, PublicationKind::ExternalInputRequired)?;
        }
        insert_event(&tx, &event)?;
        tx.commit().map_err(AppError::store)
    }

    fn request_control(
        &mut self,
        work_id: &WorkId,
        kind: ControlKind,
        created_at_unix_ms: u64,
    ) -> Result<ControlDirective, AppError> {
        let (execution_id, attempt_id): (String, String) = self
            .conn
            .query_row(
                "SELECT active_execution_id, active_attempt_id FROM works
                 WHERE id = ?1 AND status = 'running'
                   AND active_execution_id IS NOT NULL AND active_attempt_id IS NOT NULL",
                [work_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(AppError::store)?
            .ok_or_else(|| AppError::Conflict(format!("Work {work_id} has no active attempt")))?;
        self.conn
            .execute(
                "INSERT INTO control_requests
                   (work_id, kind, state, created_at_unix_ms, execution_id, attempt_id)
                 VALUES (?1, ?2, 'pending', ?3, ?4, ?5)",
                params![
                    work_id.as_str(),
                    kind.as_str(),
                    created_at_unix_ms as i64,
                    execution_id,
                    attempt_id,
                ],
            )
            .map_err(map_constraint_conflict)?;
        Ok(ControlDirective {
            request_id: self.conn.last_insert_rowid(),
            kind,
        })
    }

    fn reclaim_attempt(
        &mut self,
        work: &Work,
        event: WorkEvent,
        execution_id: &ExecutionId,
        attempt_id: &AttemptId,
    ) -> Result<(), AppError> {
        let tx = self.conn.transaction().map_err(AppError::store)?;
        let changed = tx
            .execute(
                "UPDATE works
                 SET status = 'parked', generation = generation + 1,
                     active_execution_id = NULL, active_attempt_id = NULL
                 WHERE id = ?1 AND status = 'running'
                   AND active_execution_id = ?2 AND active_attempt_id = ?3",
                params![
                    work.id().as_str(),
                    execution_id.as_str(),
                    attempt_id.as_str()
                ],
            )
            .map_err(AppError::store)?;
        if changed != 1 {
            return Err(attempt_conflict(attempt_id));
        }
        release_queue_quotas(&tx, work)?;
        tx.execute(
            "UPDATE attempts
             SET state = 'parked', terminal_reason = 'crash_reclaimed',
                 finished_at_unix_ms = ?3
             WHERE id = ?1 AND execution_id = ?2 AND state = 'active'",
            params![
                attempt_id.as_str(),
                execution_id.as_str(),
                event.created_at_unix_ms() as i64,
            ],
        )
        .map_err(AppError::store)?;
        tx.execute(
            "UPDATE control_requests SET state = 'cancelled', finished_at_unix_ms = ?2
             WHERE work_id = ?1 AND state = 'pending'",
            params![work.id().as_str(), event.created_at_unix_ms() as i64],
        )
        .map_err(AppError::store)?;
        insert_event(&tx, &event)?;
        tx.commit().map_err(AppError::store)
    }

    fn append_operator_input(
        &mut self,
        work_id: &WorkId,
        kind: OperatorInputKind,
        body: &str,
        created_at_unix_ms: u64,
    ) -> Result<OperatorInput, AppError> {
        self.conn
            .execute(
                "INSERT INTO operator_inputs (work_id, kind, body, created_at_unix_ms)
                 SELECT id, ?2, ?3, ?4 FROM works WHERE id = ?1 AND status = 'parked'",
                params![
                    work_id.as_str(),
                    kind.as_str(),
                    body,
                    created_at_unix_ms as i64,
                ],
            )
            .map_err(AppError::store)
            .and_then(|changed| {
                if changed == 1 {
                    Ok(())
                } else {
                    Err(AppError::Conflict(format!("Work {work_id} is not parked")))
                }
            })?;
        Ok(OperatorInput {
            id: self.conn.last_insert_rowid(),
            kind,
            body: body.to_owned(),
            created_at_unix_ms,
        })
    }
}

impl QueueStore for SqliteStore {
    fn capture(
        &mut self,
        request: &CaptureRequest<'_>,
        captured_at_unix_ms: u64,
    ) -> Result<Option<CaptureLease>, AppError> {
        if request.worker_id.trim().is_empty() {
            return Err(AppError::Conflict("capture worker id is empty".to_owned()));
        }
        let tx = self.conn.transaction().map_err(AppError::store)?;
        let project = request.project_id.map(ProjectId::as_str);
        let selected = tx
            .query_row(
                "SELECT w.id, w.status, w.goal, w.worker_profile, w.workspace_root,
                        w.created_at_unix_ms, w.schema_version, w.project_id,
                        w.repository, w.notification_target, w.generation
                 FROM works w
                 WHERE w.status IN ('ready', 'parked')
                   AND (?1 IS NULL OR w.project_id = ?1)
                   AND w.active_attempt_id IS NULL
                   AND NOT EXISTS (
                     SELECT 1 FROM captures c
                     WHERE c.work_id = w.id AND c.state = 'active'
                   )
                   AND NOT EXISTS (
                     SELECT 1 FROM work_relations r
                     JOIN works blocker ON blocker.id = r.from_work_id
                     WHERE r.to_work_id = w.id AND r.kind = 'blocks'
                       AND blocker.status <> 'succeeded'
                   )
                   AND NOT EXISTS (
                     SELECT 1 FROM quota_limits q
                     WHERE q.resource = 'queue:global'
                       AND (SELECT COALESCE(SUM(l.units), 0) FROM quota_leases l
                            WHERE l.resource = q.resource AND l.state = 'active')
                           >= q.unit_limit
                   )
                   AND NOT EXISTS (
                     SELECT 1 FROM quota_limits q
                     WHERE q.resource = 'project:' || w.project_id
                       AND (SELECT COALESCE(SUM(l.units), 0) FROM quota_leases l
                            WHERE l.resource = q.resource AND l.state = 'active')
                           >= q.unit_limit
                   )
                 ORDER BY CASE w.status WHEN 'ready' THEN 0 ELSE 1 END,
                          w.created_at_unix_ms, w.id
                 LIMIT 1",
                [project],
                |row| Ok((SqliteStore::read_row(row)?, row.get::<_, i64>(10)?)),
            )
            .optional()
            .map_err(AppError::store)?;
        let Some((row, generation)) = selected else {
            return Ok(None);
        };
        let work = SqliteStore::work_from_row(row)?;
        let capture_id = format!("capture-{}", uuid::Uuid::new_v4());
        tx.execute(
            "INSERT INTO captures (
                 id, work_id, project_id, worker_id, work_generation, state,
                 captured_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6)",
            params![
                capture_id,
                work.id().as_str(),
                work.attributes().project_id().as_str(),
                request.worker_id,
                generation,
                captured_at_unix_ms as i64,
            ],
        )
        .map_err(map_constraint_conflict)?;
        acquire_queue_quotas(&tx, &work)?;
        tx.commit().map_err(AppError::store)?;
        Ok(Some(CaptureLease {
            capture_id,
            work,
            worker_id: request.worker_id.to_owned(),
            generation: unsigned(generation, "capture generation")?,
            captured_at_unix_ms,
        }))
    }

    fn release_capture(&mut self, lease: &CaptureLease) -> Result<(), AppError> {
        let tx = self.conn.transaction().map_err(AppError::store)?;
        let changed = tx
            .execute(
                "UPDATE captures SET state = 'released', finished_at_unix_ms = ?1
                 WHERE id = ?2 AND work_id = ?3 AND worker_id = ?4
                   AND work_generation = ?5 AND state = 'active'",
                params![
                    lease.captured_at_unix_ms as i64,
                    lease.capture_id,
                    lease.work.id().as_str(),
                    lease.worker_id,
                    lease.generation as i64,
                ],
            )
            .map_err(AppError::store)?;
        if changed != 1 {
            return Err(AppError::Conflict("capture lease was lost".to_owned()));
        }
        release_queue_quotas(&tx, &lease.work)?;
        tx.commit().map_err(AppError::store)
    }

    fn reclaim_captures(&mut self) -> Result<usize, AppError> {
        let tx = self.conn.transaction().map_err(AppError::store)?;
        tx.execute(
            "UPDATE quota_leases SET state = 'released'
             WHERE state = 'active' AND holder IN (
               SELECT 'work:' || work_id FROM captures WHERE state = 'active'
             )",
            [],
        )
        .map_err(AppError::store)?;
        let changed = tx
            .execute(
                "UPDATE captures SET state = 'released', finished_at_unix_ms = captured_at_unix_ms
                 WHERE state = 'active'",
                [],
            )
            .map_err(AppError::store)?;
        tx.commit().map_err(AppError::store)?;
        Ok(changed)
    }
}

impl RelationStore for SqliteStore {
    fn add_relation(&mut self, relation: &WorkRelation) -> Result<(), AppError> {
        let tx = self.conn.transaction().map_err(AppError::store)?;
        let from_project: Option<String> = tx
            .query_row(
                "SELECT project_id FROM works WHERE id = ?1",
                [relation.from().as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(AppError::store)?;
        let to_project: Option<String> = tx
            .query_row(
                "SELECT project_id FROM works WHERE id = ?1",
                [relation.to().as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(AppError::store)?;
        match (from_project, to_project) {
            (Some(from), Some(to)) if from == to => {}
            (Some(_), Some(_)) => {
                return Err(AppError::Conflict(
                    "Work relations cannot cross project boundaries".to_owned(),
                ));
            }
            _ => return Err(AppError::Conflict("related Work was not found".to_owned())),
        }
        tx.execute(
            "INSERT INTO work_relations (from_work_id, to_work_id, kind)
             VALUES (?1, ?2, ?3)
             ON CONFLICT DO NOTHING",
            params![
                relation.from().as_str(),
                relation.to().as_str(),
                relation.kind().as_str(),
            ],
        )
        .map_err(map_constraint_conflict)?;
        tx.commit().map_err(AppError::store)
    }

    fn relations(&self, work_id: &WorkId) -> Result<Vec<WorkRelation>, AppError> {
        relations_connection(&self.conn, work_id)
    }
}

impl SqliteObserver {
    /// Read typed Work relation data without granting an observer mutation access.
    pub fn relations(&self, work_id: &WorkId) -> Result<Vec<WorkRelation>, AppError> {
        relations_connection(&self.conn, work_id)
    }
}

fn relations_connection(
    connection: &Connection,
    work_id: &WorkId,
) -> Result<Vec<WorkRelation>, AppError> {
    let mut statement = connection
        .prepare(
            "SELECT from_work_id, to_work_id, kind FROM work_relations
             WHERE from_work_id = ?1 OR to_work_id = ?1
             ORDER BY from_work_id, to_work_id, kind",
        )
        .map_err(AppError::store)?;
    let rows = statement
        .query_map([work_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(AppError::store)?;
    let mut relations = Vec::new();
    for row in rows {
        let (from, to, kind) = row.map_err(AppError::store)?;
        relations.push(WorkRelation::new(
            WorkId::parse(from)?,
            WorkId::parse(to)?,
            RelationKind::from_str(&kind)?,
        )?);
    }
    Ok(relations)
}

impl QuotaStore for SqliteStore {
    fn configure_quota(
        &mut self,
        resource: &str,
        limit: u32,
        expected_generation: Option<u64>,
    ) -> Result<u64, AppError> {
        let tx = self.conn.transaction().map_err(AppError::store)?;
        let current: Option<u64> = tx
            .query_row(
                "SELECT generation FROM quota_limits WHERE resource = ?1",
                [resource],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(AppError::store)?
            .map(|value| unsigned(value, "quota generation"))
            .transpose()?;
        let next = match (current, expected_generation) {
            (None, None) => {
                tx.execute(
                    "INSERT INTO quota_limits (resource, unit_limit, generation)
                     VALUES (?1, ?2, 0)",
                    params![resource, i64::from(limit)],
                )
                .map_err(map_constraint_conflict)?;
                0
            }
            (Some(current), Some(expected)) if current == expected => {
                let next = current.saturating_add(1);
                tx.execute(
                    "UPDATE quota_limits SET unit_limit = ?1, generation = ?2
                     WHERE resource = ?3 AND generation = ?4",
                    params![i64::from(limit), next as i64, resource, expected as i64],
                )
                .map_err(AppError::store)?;
                next
            }
            _ => return Err(AppError::Conflict("quota context changed".to_owned())),
        };
        tx.commit().map_err(AppError::store)?;
        Ok(next)
    }

    fn acquire_quota(
        &mut self,
        resource: &str,
        holder: &str,
        units: u32,
    ) -> Result<QuotaLease, AppError> {
        if units == 0 {
            return Err(AppError::Conflict(
                "quota units must be positive".to_owned(),
            ));
        }
        let tx = self.conn.transaction().map_err(AppError::store)?;
        let limit: i64 = tx
            .query_row(
                "SELECT unit_limit FROM quota_limits WHERE resource = ?1",
                [resource],
                |row| row.get(0),
            )
            .optional()
            .map_err(AppError::store)?
            .ok_or_else(|| AppError::Conflict(format!("quota {resource} is not configured")))?;
        let used: i64 = tx
            .query_row(
                "SELECT COALESCE(SUM(units), 0) FROM quota_leases
                 WHERE resource = ?1 AND state = 'active'",
                [resource],
                |row| row.get(0),
            )
            .map_err(AppError::store)?;
        if used.saturating_add(i64::from(units)) > limit {
            return Err(AppError::Conflict(format!(
                "quota {resource} has insufficient capacity"
            )));
        }
        let lease_id = format!("quota-{}", uuid::Uuid::new_v4());
        tx.execute(
            "INSERT INTO quota_leases (id, resource, holder, units, state)
             VALUES (?1, ?2, ?3, ?4, 'active')",
            params![lease_id, resource, holder, i64::from(units)],
        )
        .map_err(map_constraint_conflict)?;
        tx.commit().map_err(AppError::store)?;
        Ok(QuotaLease {
            lease_id,
            resource: resource.to_owned(),
            holder: holder.to_owned(),
            units,
        })
    }

    fn release_quota(&mut self, lease: &QuotaLease) -> Result<(), AppError> {
        let changed = self
            .conn
            .execute(
                "UPDATE quota_leases SET state = 'released'
                 WHERE id = ?1 AND resource = ?2 AND holder = ?3
                   AND units = ?4 AND state = 'active'",
                params![
                    lease.lease_id,
                    lease.resource,
                    lease.holder,
                    i64::from(lease.units),
                ],
            )
            .map_err(AppError::store)?;
        if changed != 1 {
            return Err(AppError::Conflict("quota lease was lost".to_owned()));
        }
        Ok(())
    }
}

impl InboundStore for SqliteStore {
    fn put_inbound(
        &mut self,
        source_id: &str,
        record_id: &str,
        work: &Work,
        event: WorkEvent,
    ) -> Result<bool, AppError> {
        let tx = self.conn.transaction().map_err(AppError::store)?;
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM inbound_receipts
                   WHERE source_id = ?1 AND record_id = ?2
                 )",
                params![source_id, record_id],
                |row| row.get(0),
            )
            .map_err(AppError::store)?;
        if exists {
            return Ok(false);
        }
        put_in_tx(&tx, work, &event)?;
        tx.execute(
            "INSERT INTO inbound_receipts (source_id, record_id, work_id)
             VALUES (?1, ?2, ?3)",
            params![source_id, record_id, work.id().as_str()],
        )
        .map_err(map_constraint_conflict)?;
        tx.commit().map_err(AppError::store)?;
        Ok(true)
    }
}

impl PublicationStore for SqliteStore {
    fn pending_publications(&self, limit: usize) -> Result<Vec<Publication>, AppError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut statement = self
            .conn
            .prepare(
                "SELECT id, kind, work_id, project_id, target, status,
                        created_at_unix_ms, attempt_count
                 FROM publications WHERE state = 'pending'
                 ORDER BY id LIMIT ?1",
            )
            .map_err(AppError::store)?;
        let rows = statement
            .query_map([limit], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            })
            .map_err(AppError::store)?;
        let mut publications = Vec::new();
        for row in rows {
            let (id, kind, work_id, project_id, target, status, created_at, attempts) =
                row.map_err(AppError::store)?;
            let kind = match kind.as_str() {
                "work_transition" => PublicationKind::WorkTransition,
                "external_input_required" => PublicationKind::ExternalInputRequired,
                _ => return Err(AppError::store("unknown publication kind")),
            };
            publications.push(Publication {
                publication_id: id,
                kind,
                work_id: WorkId::parse(work_id)?,
                project_id: ProjectId::parse(project_id)?,
                target,
                status: WorkStatus::from_str(&status)?,
                created_at_unix_ms: unsigned(created_at, "publication time")?,
                attempt_count: u32::try_from(attempts)
                    .map_err(|_| AppError::store("publication attempt count overflow"))?,
            });
        }
        Ok(publications)
    }

    fn record_publication_attempt(
        &mut self,
        publication_id: i64,
        delivered: bool,
        attempted_at_unix_ms: u64,
        error: Option<&str>,
    ) -> Result<(), AppError> {
        let state = if delivered { "delivered" } else { "failed" };
        let scrubbed_error = error.map(|_| "delivery_failed");
        let changed = self
            .conn
            .execute(
                "UPDATE publications
                 SET state = ?1, attempt_count = attempt_count + 1,
                     last_attempt_at_unix_ms = ?2, last_error = ?3
                 WHERE id = ?4 AND state = 'pending'",
                params![
                    state,
                    attempted_at_unix_ms as i64,
                    scrubbed_error,
                    publication_id,
                ],
            )
            .map_err(AppError::store)?;
        if changed != 1 {
            return Err(AppError::Conflict(format!(
                "publication {publication_id} is not pending"
            )));
        }
        Ok(())
    }
}

fn acquire_queue_quotas(tx: &Transaction<'_>, work: &Work) -> Result<(), AppError> {
    let holder = format!("work:{}", work.id());
    let project_resource = format!("project:{}", work.attributes().project_id());
    for resource in ["queue:global", project_resource.as_str()] {
        let limit: Option<i64> = tx
            .query_row(
                "SELECT unit_limit FROM quota_limits WHERE resource = ?1",
                [resource],
                |row| row.get(0),
            )
            .optional()
            .map_err(AppError::store)?;
        let Some(limit) = limit else { continue };
        let used: i64 = tx
            .query_row(
                "SELECT COALESCE(SUM(units), 0) FROM quota_leases
                 WHERE resource = ?1 AND state = 'active'",
                [resource],
                |row| row.get(0),
            )
            .map_err(AppError::store)?;
        if used >= limit {
            return Err(AppError::Conflict(format!(
                "quota {resource} has insufficient capacity"
            )));
        }
        tx.execute(
            "INSERT INTO quota_leases (id, resource, holder, units, state)
             VALUES (?1, ?2, ?3, 1, 'active')",
            params![format!("quota-{}", uuid::Uuid::new_v4()), resource, holder],
        )
        .map_err(map_constraint_conflict)?;
    }
    Ok(())
}

fn release_queue_quotas(conn: &Connection, work: &Work) -> Result<(), AppError> {
    let holder = format!("work:{}", work.id());
    conn.execute(
        "UPDATE quota_leases SET state = 'released'
         WHERE holder = ?1 AND state = 'active'
           AND (resource = 'queue:global' OR resource = ?2)",
        params![
            holder,
            format!("project:{}", work.attributes().project_id()),
        ],
    )
    .map_err(AppError::store)?;
    Ok(())
}

fn resolve_controls(
    tx: &Transaction<'_>,
    work_id: &WorkId,
    handled_id: Option<i64>,
    finished_at_unix_ms: u64,
) -> Result<(), AppError> {
    if let Some(id) = handled_id {
        let changed = tx
            .execute(
                "UPDATE control_requests SET state = 'handled', finished_at_unix_ms = ?1
                 WHERE id = ?2 AND work_id = ?3 AND state = 'pending'",
                params![finished_at_unix_ms as i64, id, work_id.as_str()],
            )
            .map_err(AppError::store)?;
        if changed != 1 {
            return Err(AppError::Conflict(format!(
                "control request {id} is not pending for Work {work_id}"
            )));
        }
    }
    tx.execute(
        "UPDATE control_requests SET state = 'cancelled', finished_at_unix_ms = ?2
         WHERE work_id = ?1 AND state = 'pending'",
        params![work_id.as_str(), finished_at_unix_ms as i64],
    )
    .map_err(AppError::store)?;
    Ok(())
}

fn migrate(conn: &Connection) -> Result<(), AppError> {
    let current: i32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(AppError::store)?;
    if current == 1 || current > STORE_USER_VERSION {
        return Err(AppError::from(
            workengine_domain::DomainError::UnsupportedSchemaVersion(current as u32),
        ));
    }
    if current == 0 {
        conn.execute_batch(MIGRATION_1).map_err(AppError::store)?;
        conn.execute_batch(MIGRATION_2).map_err(AppError::store)?;
        conn.execute_batch(MIGRATION_3).map_err(AppError::store)?;
        conn.execute_batch(MIGRATION_4).map_err(AppError::store)?;
        conn.execute_batch(MIGRATION_5).map_err(AppError::store)?;
        conn.execute_batch(MIGRATION_6).map_err(AppError::store)?;
        conn.pragma_update(None, "user_version", STORE_USER_VERSION)
            .map_err(AppError::store)?;
    } else if current == 2 {
        conn.execute_batch(MIGRATION_3).map_err(AppError::store)?;
        conn.execute_batch(MIGRATION_4).map_err(AppError::store)?;
        conn.execute_batch(MIGRATION_5).map_err(AppError::store)?;
        conn.execute_batch(MIGRATION_6).map_err(AppError::store)?;
        conn.pragma_update(None, "user_version", STORE_USER_VERSION)
            .map_err(AppError::store)?;
    } else if current == 3 {
        conn.execute_batch(MIGRATION_4).map_err(AppError::store)?;
        conn.execute_batch(MIGRATION_5).map_err(AppError::store)?;
        conn.execute_batch(MIGRATION_6).map_err(AppError::store)?;
        conn.pragma_update(None, "user_version", STORE_USER_VERSION)
            .map_err(AppError::store)?;
    } else if current == 4 {
        conn.execute_batch(MIGRATION_5).map_err(AppError::store)?;
        conn.execute_batch(MIGRATION_6).map_err(AppError::store)?;
        conn.pragma_update(None, "user_version", STORE_USER_VERSION)
            .map_err(AppError::store)?;
    } else if current == 5 {
        conn.execute_batch(MIGRATION_6).map_err(AppError::store)?;
        conn.pragma_update(None, "user_version", STORE_USER_VERSION)
            .map_err(AppError::store)?;
    }
    Ok(())
}

fn restrict_directory(path: &Path) -> Result<(), AppError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(AppError::store)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn restrict_file(path: &Path) -> Result<(), AppError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(AppError::store)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn put_in_tx(tx: &Transaction<'_>, work: &Work, event: &WorkEvent) -> Result<(), AppError> {
    tx.execute(
        "INSERT INTO works (
             id, status, goal, worker_profile, workspace_root, created_at_unix_ms,
             schema_version, project_id, repository, notification_target
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(id) DO UPDATE SET
            status = excluded.status,
            goal = excluded.goal,
            worker_profile = excluded.worker_profile,
            workspace_root = excluded.workspace_root,
            project_id = excluded.project_id,
            repository = excluded.repository,
            notification_target = excluded.notification_target,
            generation = works.generation + 1
         WHERE works.active_attempt_id IS NULL
           AND works.goal = excluded.goal
           AND works.worker_profile = excluded.worker_profile
           AND works.project_id = excluded.project_id
           AND works.repository IS excluded.repository
           AND works.notification_target IS excluded.notification_target",
        params![
            work.id().as_str(),
            work.status().as_str(),
            work.attributes().goal(),
            work.attributes().worker_profile(),
            work.workspace_root(),
            work.created_at_unix_ms() as i64,
            WORK_SCHEMA_VERSION as i64,
            work.attributes().project_id().as_str(),
            work.attributes().repository(),
            work.attributes().notification_target(),
        ],
    )
    .map_err(AppError::store)
    .and_then(|changed| {
        if changed == 1 {
            Ok(())
        } else {
            Err(AppError::Conflict(format!(
                "Work {} has an active attempt or changed immutable attributes",
                work.id()
            )))
        }
    })?;
    insert_event(tx, event)?;
    Ok(())
}

fn insert_event(tx: &Transaction<'_>, event: &WorkEvent) -> Result<(), AppError> {
    let payload = encode_event(event)?;
    tx.execute(
        "INSERT INTO events (work_id, payload) VALUES (?1, ?2)",
        params![event.work_id().as_str(), payload],
    )
    .map_err(AppError::store)?;
    enqueue_publication(tx, event, PublicationKind::WorkTransition)?;
    Ok(())
}

fn enqueue_publication(
    tx: &Transaction<'_>,
    event: &WorkEvent,
    kind: PublicationKind,
) -> Result<(), AppError> {
    let target: Option<(String, String)> = tx
        .query_row(
            "SELECT project_id, notification_target FROM works
             WHERE id = ?1 AND notification_target IS NOT NULL",
            [event.work_id().as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(AppError::store)?;
    let Some((project_id, target)) = target else {
        return Ok(());
    };
    tx.execute(
        "INSERT INTO publications (
             kind, work_id, project_id, target, status, state, created_at_unix_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6)",
        params![
            kind.as_str(),
            event.work_id().as_str(),
            project_id,
            target,
            event.to().as_str(),
            event.created_at_unix_ms() as i64,
        ],
    )
    .map_err(AppError::store)?;
    Ok(())
}

fn encode_execution_spec(spec: &ExecutionSpec) -> Result<String, AppError> {
    let secret_refs = spec
        .secret_refs()
        .iter()
        .map(|reference| SecretRefPayload {
            name: reference.name(),
            source: reference.source().reference(),
        })
        .collect();
    serde_json::to_string(&ExecutionSpecPayload {
        schema_version: spec.schema_version(),
        work_id: spec.work_id().as_str(),
        worker_profile: spec.worker_profile(),
        worker_config_digest: spec.worker_config_digest().as_str(),
        runtime_kind: spec.runtime_kind().as_str(),
        runtime_digest: spec.runtime_digest().as_str(),
        wall_clock_budget_ms: spec.wall_clock_budget_ms(),
        retry_limit: spec.retry_limit(),
        channel_policy: spec.channel_policy().as_str(),
        secret_refs,
    })
    .map_err(AppError::store)
}

fn encode_confirmed_outcome(outcome: &ConfirmedOutcome) -> Result<String, AppError> {
    serde_json::to_string(&ConfirmedOutcomePayload {
        schema_version: outcome.schema_version(),
        work_id: outcome.work_id().as_str(),
        execution_id: outcome.execution_id().as_str(),
        attempt_id: outcome.attempt_id().as_str(),
        kind: outcome.outcome().kind().as_str(),
        worker_profile: outcome.outcome().worker_profile(),
        confirmed_at_unix_ms: outcome.confirmed_at_unix_ms(),
    })
    .map_err(AppError::store)
}

fn attempt_conflict(attempt_id: &AttemptId) -> AppError {
    AppError::Conflict(format!(
        "attempt {attempt_id} does not own the active lease"
    ))
}

fn map_constraint_conflict(error: rusqlite::Error) -> AppError {
    if matches!(error, rusqlite::Error::SqliteFailure(_, _)) {
        AppError::Conflict(error.to_string())
    } else {
        AppError::store(error)
    }
}

fn encode_event(event: &WorkEvent) -> Result<String, AppError> {
    let payload = EventPayload {
        schema_version: event.schema_version(),
        work_id: event.work_id().to_string(),
        kind: event.kind().as_str().to_owned(),
        from: event.from().map(|s| s.as_str().to_owned()),
        to: event.to().as_str().to_owned(),
        goal: event.attributes().map(|a| a.goal().to_owned()),
        worker_profile: event.attributes().map(|a| a.worker_profile().to_owned()),
        project_id: event
            .attributes()
            .map(|attributes| attributes.project_id().to_string()),
        repository: event
            .attributes()
            .and_then(|attributes| attributes.repository().map(str::to_owned)),
        notification_target: event
            .attributes()
            .and_then(|attributes| attributes.notification_target().map(str::to_owned)),
        outcome_kind: event.outcome_kind().map(|k| k.as_str().to_owned()),
        workspace_root: event.workspace_root().map(str::to_owned),
        created_at_unix_ms: event.created_at_unix_ms(),
    };
    serde_json::to_string(&payload).map_err(AppError::store)
}

fn decode_event(payload: &str) -> Result<WorkEvent, AppError> {
    let parsed: EventPayload = serde_json::from_str(payload).map_err(AppError::outcome_schema)?;
    if parsed.schema_version != EVENT_SCHEMA_VERSION {
        return Err(AppError::from(
            workengine_domain::DomainError::UnsupportedSchemaVersion(parsed.schema_version),
        ));
    }
    let attributes = match (parsed.goal, parsed.worker_profile) {
        (Some(goal), Some(profile)) => Some(WorkAttributes::scoped(
            goal,
            profile,
            parsed
                .project_id
                .map(ProjectId::parse)
                .transpose()?
                .unwrap_or_else(ProjectId::default_project),
            parsed.repository,
            parsed.notification_target,
        )?),
        (None, None) => None,
        _ => return Err(AppError::outcome_schema("event attributes incomplete")),
    };
    WorkEvent::restore(
        parsed.schema_version,
        WorkId::parse(parsed.work_id)?,
        EventKind::from_str(&parsed.kind)?,
        parsed.from.map(|s| WorkStatus::from_str(&s)).transpose()?,
        WorkStatus::from_str(&parsed.to)?,
        attributes,
        parsed
            .outcome_kind
            .map(|s| OutcomeKind::from_str(&s))
            .transpose()?,
        parsed.workspace_root,
        parsed.created_at_unix_ms,
    )
    .map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use workengine_application::AttemptClaim;
    use workengine_domain::{
        AttemptId, CONFIRMED_OUTCOME_SCHEMA_VERSION, ChannelPolicy, ConfirmedOutcome,
        ContentDigest, EXECUTION_SPEC_SCHEMA_VERSION, ExecutionId, ExecutionSpec,
        OUTCOME_SCHEMA_VERSION, Outcome, OutcomeKind, RuntimeKind, replay,
    };

    fn store() -> (SqliteStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (SqliteStore::open(dir.path()).unwrap(), dir)
    }

    fn sample() -> Work {
        Work::new(
            WorkId::parse("work-1").unwrap(),
            WorkAttributes::new("goal", "stub").unwrap(),
            7,
        )
        .unwrap()
    }

    fn scoped_sample(id: &str, project: &str, target: Option<&str>) -> Work {
        Work::new(
            WorkId::parse(id).unwrap(),
            WorkAttributes::scoped(
                format!("goal-{id}"),
                "stub",
                ProjectId::parse(project).unwrap(),
                Some(format!("repo:{project}")),
                target.map(str::to_owned),
            )
            .unwrap(),
            7,
        )
        .unwrap()
    }

    fn execution_spec(work: &Work) -> ExecutionSpec {
        execution_spec_with_retry(work, 0)
    }

    fn execution_spec_with_retry(work: &Work, retry_limit: u32) -> ExecutionSpec {
        ExecutionSpec::new(
            EXECUTION_SPEC_SCHEMA_VERSION,
            work.id().clone(),
            work.attributes().worker_profile(),
            ContentDigest::parse(format!("sha256:{}", "1".repeat(64))).unwrap(),
            RuntimeKind::Stub,
            ContentDigest::parse(format!("sha256:{}", "2".repeat(64))).unwrap(),
            5_000,
            retry_limit,
            if retry_limit == 0 {
                ChannelPolicy::Fail
            } else {
                ChannelPolicy::RetryThenFail
            },
            Vec::new(),
        )
        .unwrap()
    }

    #[test]
    fn put_is_atomic_when_event_insert_fails() {
        let (mut store, _dir) = store();
        let work = sample();
        store.put(&work, WorkEvent::created(&work)).unwrap();
        store
            .conn
            .execute_batch(
                "CREATE TRIGGER fail_events BEFORE INSERT ON events
                 BEGIN SELECT RAISE(ABORT, 'boom'); END;",
            )
            .unwrap();
        let mut running = store.get(work.id()).unwrap().unwrap();
        let from = running.status();
        running.start().unwrap();
        let err = store
            .put(&running, WorkEvent::started(&running, from, 8))
            .unwrap_err();
        assert!(matches!(err, AppError::Store(_)));
        let loaded = store.get(work.id()).unwrap().unwrap();
        assert_eq!(loaded.status(), WorkStatus::Ready);
        assert_eq!(store.events(work.id()).unwrap().len(), 1);
    }

    #[test]
    fn replay_matches_snapshot() {
        let (mut store, _dir) = store();
        let mut work = sample();
        store.put(&work, WorkEvent::created(&work)).unwrap();
        let from = work.status();
        work.start().unwrap();
        work.bind_workspace("/ws/work-1").unwrap();
        store
            .put(&work, WorkEvent::started(&work, from, 8))
            .unwrap();
        let outcome = Outcome::new(OUTCOME_SCHEMA_VERSION, OutcomeKind::Succeeded, "stub").unwrap();
        let from = work.status();
        work.complete(&outcome).unwrap();
        store
            .put(
                &work,
                WorkEvent::completed(&work, from, OutcomeKind::Succeeded, 9),
            )
            .unwrap();
        let snapshot = store.get(work.id()).unwrap().unwrap();
        let replayed = replay(&store.events(work.id()).unwrap()).unwrap();
        assert_eq!(snapshot.status(), replayed.status());
        assert_eq!(snapshot.workspace_root(), replayed.workspace_root());
        assert_eq!(snapshot.status(), WorkStatus::Succeeded);
    }

    #[test]
    fn event_schema_file_matches_closed_kinds() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../../schemas/event.json")).unwrap();
        let kinds: Vec<&str> = schema["properties"]["kind"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let rust: Vec<&str> = EventKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(kinds, rust);
        let statuses: Vec<&str> = schema["properties"]["to"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let domain: Vec<&str> = WorkStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(statuses, domain);
        assert_eq!(schema["properties"]["schemaVersion"]["const"], 1);
    }

    #[test]
    fn open_sets_current_store_user_version() {
        let (store, _dir) = store();
        let version: i32 = store
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, STORE_USER_VERSION);
    }

    #[test]
    fn reopen_of_current_version_is_identity() {
        let dir = tempfile::tempdir().unwrap();
        let mut first = SqliteStore::open(dir.path()).unwrap();
        let work = sample();
        first.put(&work, WorkEvent::created(&work)).unwrap();
        drop(first);
        let second = SqliteStore::open(dir.path()).unwrap();
        let loaded = second.get(work.id()).unwrap().unwrap();
        assert_eq!(loaded.status(), WorkStatus::Ready);
        let version: i32 = second
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, STORE_USER_VERSION);
    }

    #[test]
    fn empty_v0_file_with_tables_initializes_to_current_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workengine.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(MIGRATION_1).unwrap();
        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 0);
        drop(conn);
        let store = SqliteStore::open(dir.path()).unwrap();
        let version: i32 = store
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, STORE_USER_VERSION);
    }

    #[test]
    fn v2_store_migrates_to_active_attempt_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workengine.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(MIGRATION_1).unwrap();
        conn.execute_batch(MIGRATION_2).unwrap();
        conn.pragma_update(None, "user_version", 2).unwrap();
        drop(conn);

        let store = SqliteStore::open(dir.path()).unwrap();
        let version: i32 = store
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, STORE_USER_VERSION);
        let columns: Vec<String> = store
            .conn
            .prepare("PRAGMA table_info(works)")
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(columns.iter().any(|column| column == "active_attempt_id"));
    }

    #[test]
    fn v5_store_migrates_existing_work_into_default_project() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workengine.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(MIGRATION_1).unwrap();
        conn.execute_batch(MIGRATION_2).unwrap();
        conn.execute_batch(MIGRATION_3).unwrap();
        conn.execute_batch(MIGRATION_4).unwrap();
        conn.execute_batch(MIGRATION_5).unwrap();
        conn.execute(
            "INSERT INTO works (
                id, status, goal, worker_profile, created_at_unix_ms,
                schema_version, generation
             ) VALUES ('old', 'ready', 'goal', 'stub', 1, 1, 0)",
            [],
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 5).unwrap();
        drop(conn);

        let store = SqliteStore::open(dir.path()).unwrap();
        let old = store.get(&WorkId::parse("old").unwrap()).unwrap().unwrap();
        assert_eq!(old.attributes().project_id().as_str(), "default");
    }

    #[test]
    fn unknown_store_user_version_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workengine.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", STORE_USER_VERSION + 1)
            .unwrap();
        drop(conn);
        let err = match SqliteStore::open(dir.path()) {
            Ok(_) => panic!("expected unknown store version to fail"),
            Err(err) => err,
        };
        assert!(matches!(
            err,
            AppError::Domain(workengine_domain::DomainError::UnsupportedSchemaVersion(version))
                if version == (STORE_USER_VERSION + 1) as u32
        ));
    }

    #[test]
    fn active_attempt_lease_rejects_a_second_start_and_foreign_confirmation() {
        let (mut store, _dir) = store();
        let work = sample();
        store.put(&work, WorkEvent::created(&work)).unwrap();
        let spec = execution_spec(&work);
        let execution_id = ExecutionId::parse("execution-1").unwrap();
        let attempt_id = AttemptId::parse("attempt-1").unwrap();
        let mut running = work.clone();
        running.start().unwrap();
        running.bind_workspace("/workspace/work-1").unwrap();
        store
            .claim_attempt(&AttemptClaim {
                execution_id: &execution_id,
                attempt_id: &attempt_id,
                spec: &spec,
                work: &running,
                event: WorkEvent::started(&running, WorkStatus::Ready, 8),
                started_at_unix_ms: 8,
                capture: None,
            })
            .unwrap();

        let second = store.claim_attempt(&AttemptClaim {
            execution_id: &ExecutionId::parse("execution-2").unwrap(),
            attempt_id: &AttemptId::parse("attempt-2").unwrap(),
            spec: &spec,
            work: &running,
            event: WorkEvent::started(&running, WorkStatus::Ready, 9),
            started_at_unix_ms: 9,
            capture: None,
        });
        assert!(matches!(second, Err(AppError::Conflict(_))));

        let outcome = Outcome::new(OUTCOME_SCHEMA_VERSION, OutcomeKind::Succeeded, "stub").unwrap();
        let foreign = ConfirmedOutcome::new(
            CONFIRMED_OUTCOME_SCHEMA_VERSION,
            execution_id,
            AttemptId::parse("foreign-attempt").unwrap(),
            &spec,
            outcome,
            10,
        )
        .unwrap();
        let mut succeeded = running;
        succeeded.complete(foreign.outcome()).unwrap();
        let error = store.confirm_attempt(
            &succeeded,
            WorkEvent::completed(&succeeded, WorkStatus::Running, OutcomeKind::Succeeded, 10),
            &foreign,
        );
        assert!(matches!(error, Err(AppError::Conflict(_))));
        assert_eq!(
            store.get(work.id()).unwrap().unwrap().status(),
            WorkStatus::Running
        );
        assert_eq!(store.events(work.id()).unwrap().len(), 2);
    }

    #[test]
    fn execution_projection_is_typed_redacted_and_lease_scoped() {
        let (mut store, _dir) = store();
        let work = sample();
        store.put(&work, WorkEvent::created(&work)).unwrap();
        let spec = execution_spec_with_retry(&work, 1);
        let execution_id = ExecutionId::parse("execution-observed").unwrap();
        let attempt_id = AttemptId::parse("attempt-observed").unwrap();
        let retry_attempt_id = AttemptId::parse("attempt-retry").unwrap();
        let mut running = work.clone();
        running.start().unwrap();
        running.bind_workspace("/workspace/work-1").unwrap();
        store
            .claim_attempt(&AttemptClaim {
                execution_id: &execution_id,
                attempt_id: &attempt_id,
                spec: &spec,
                work: &running,
                event: WorkEvent::started(&running, WorkStatus::Ready, 8),
                started_at_unix_ms: 8,
                capture: None,
            })
            .unwrap();
        store
            .record_process_event(
                work.id(),
                &execution_id,
                &attempt_id,
                ProcessEvent::Spawned,
                9,
            )
            .unwrap();
        for observed_at in [10, 11] {
            store
                .record_process_event(
                    work.id(),
                    &execution_id,
                    &attempt_id,
                    ProcessEvent::ChildStdout,
                    observed_at,
                )
                .unwrap();
        }
        store
            .heartbeat_attempt(work.id(), &execution_id, &attempt_id, 12)
            .unwrap();
        store
            .retry_attempt(&execution_id, &attempt_id, &retry_attempt_id, 13)
            .unwrap();
        let stale_before_confirmation = store.record_process_event(
            work.id(),
            &execution_id,
            &attempt_id,
            ProcessEvent::ChildStderr,
            13,
        );
        assert!(matches!(
            stale_before_confirmation,
            Err(AppError::Conflict(_))
        ));
        store
            .record_process_event(
                work.id(),
                &execution_id,
                &retry_attempt_id,
                ProcessEvent::Spawned,
                13,
            )
            .unwrap();
        store
            .record_process_event(
                work.id(),
                &execution_id,
                &retry_attempt_id,
                ProcessEvent::Exited,
                14,
            )
            .unwrap();
        let outcome = Outcome::new(OUTCOME_SCHEMA_VERSION, OutcomeKind::Succeeded, "stub").unwrap();
        let confirmed = ConfirmedOutcome::new(
            CONFIRMED_OUTCOME_SCHEMA_VERSION,
            execution_id.clone(),
            retry_attempt_id.clone(),
            &spec,
            outcome,
            15,
        )
        .unwrap();
        let mut succeeded = running;
        succeeded.complete(confirmed.outcome()).unwrap();
        store
            .confirm_attempt(
                &succeeded,
                WorkEvent::completed(&succeeded, WorkStatus::Running, OutcomeKind::Succeeded, 15),
                &confirmed,
            )
            .unwrap();

        let executions = store.executions(work.id()).unwrap();
        assert_eq!(executions.len(), 1);
        let execution = &executions[0];
        assert_eq!(execution.spec.runtime_kind.as_deref(), Some("stub"));
        assert_eq!(execution.spec.wall_clock_budget_ms, 5_000);
        assert_eq!(execution.spec.retry_limit, 1);
        assert_eq!(execution.attempts.len(), 2);
        let first_attempt = &execution.attempts[0];
        assert_eq!(first_attempt.state, AttemptState::Retried);
        assert_eq!(first_attempt.retry_ordinal, 0);
        assert_eq!(
            first_attempt.terminal_reason.as_deref(),
            Some("channel_retry")
        );
        let stdout = first_attempt
            .process_records
            .iter()
            .find(|record| record.event == ProcessEvent::ChildStdout)
            .unwrap();
        assert_eq!(stdout.occurrences, 2);
        assert!(stdout.payload_redacted);

        let attempt = &execution.attempts[1];
        assert_eq!(attempt.state, AttemptState::Confirmed);
        assert_eq!(attempt.retry_ordinal, 1);
        assert_eq!(attempt.last_heartbeat_at_unix_ms, 15);
        assert_eq!(attempt.terminal_reason.as_deref(), Some("succeeded"));
        assert_eq!(
            attempt.confirmed_outcome.as_ref().unwrap().kind,
            OutcomeKind::Succeeded
        );

        let stale = store.record_process_event(
            work.id(),
            &execution_id,
            &retry_attempt_id,
            ProcessEvent::ChildStderr,
            16,
        );
        assert!(matches!(stale, Err(AppError::Conflict(_))));
    }

    #[test]
    fn live_control_is_lease_scoped_and_park_requires_checkpoint() {
        let (mut store, _dir) = store();
        let work = sample();
        store.put(&work, WorkEvent::created(&work)).unwrap();
        let spec = execution_spec(&work);
        let execution_id = ExecutionId::parse("execution-control").unwrap();
        let attempt_id = AttemptId::parse("attempt-control").unwrap();
        let mut running = work.clone();
        running.start().unwrap();
        store
            .claim_attempt(&AttemptClaim {
                execution_id: &execution_id,
                attempt_id: &attempt_id,
                spec: &spec,
                work: &running,
                event: WorkEvent::started(&running, WorkStatus::Ready, 8),
                started_at_unix_ms: 8,
                capture: None,
            })
            .unwrap();
        let directive = store
            .request_control(work.id(), ControlKind::Park, 9)
            .unwrap();
        assert_eq!(
            store
                .control_directive(work.id(), &execution_id, &attempt_id)
                .unwrap(),
            Some(directive)
        );
        let mut parked = running.clone();
        parked.park().unwrap();
        let rejected = store.park_attempt_with_checkpoint(
            &parked,
            WorkEvent::parked(&parked, WorkStatus::Running, 10),
            &execution_id,
            &attempt_id,
            false,
            Some(directive.request_id),
        );
        assert!(matches!(rejected, Err(AppError::Conflict(_))));
        store
            .record_checkpoint(work.id(), &execution_id, &attempt_id, 10)
            .unwrap();
        store
            .park_attempt_with_checkpoint(
                &parked,
                WorkEvent::parked(&parked, WorkStatus::Running, 10),
                &execution_id,
                &attempt_id,
                true,
                Some(directive.request_id),
            )
            .unwrap();
        let observed = store.executions(work.id()).unwrap();
        assert_eq!(observed[0].attempts[0].state, AttemptState::Parked);
        assert!(observed[0].attempts[0].checkpoint_recorded);
        assert!(
            store
                .control_directive(work.id(), &execution_id, &attempt_id)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn v1_store_is_rejected_without_deleting_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workengine.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(MIGRATION_1).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        drop(conn);
        let err = match SqliteStore::open(dir.path()) {
            Ok(_) => panic!("expected v1 store to fail"),
            Err(err) => err,
        };
        assert!(matches!(
            err,
            AppError::Domain(workengine_domain::DomainError::UnsupportedSchemaVersion(1))
        ));
        assert!(path.exists());
    }

    #[test]
    fn journal_is_append_only() {
        let (mut store, _dir) = store();
        let work = sample();
        store.put(&work, WorkEvent::created(&work)).unwrap();
        let err = store.conn.execute("DELETE FROM events", []).unwrap_err();
        assert!(err.to_string().contains("append-only"));
    }

    #[test]
    fn event_cursor_is_strictly_after_and_monotonic() {
        let (mut store, _dir) = store();
        let work = sample();
        store.put(&work, WorkEvent::created(&work)).unwrap();
        let all = store.events_after(0, Some(work.id())).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(store.events_after(all[0].seq, None).unwrap().len(), 0);
    }

    #[test]
    fn project_scoped_capture_is_exclusive_and_respects_blocks() {
        let (mut store, _dir) = store();
        let blocker = scoped_sample("blocker", "project-a", None);
        let blocked = scoped_sample("blocked", "project-a", None);
        let independent = scoped_sample("independent", "project-b", None);
        for work in [&blocker, &blocked, &independent] {
            store.put(work, WorkEvent::created(work)).unwrap();
        }
        store
            .add_relation(
                &WorkRelation::new(
                    blocker.id().clone(),
                    blocked.id().clone(),
                    RelationKind::Blocks,
                )
                .unwrap(),
            )
            .unwrap();

        let project_a = ProjectId::parse("project-a").unwrap();
        let first = store
            .capture(
                &CaptureRequest {
                    project_id: Some(&project_a),
                    worker_id: "worker-a",
                },
                8,
            )
            .unwrap()
            .unwrap();
        assert_eq!(first.work.id(), blocker.id());
        assert!(
            store
                .capture(
                    &CaptureRequest {
                        project_id: Some(&project_a),
                        worker_id: "worker-b",
                    },
                    8,
                )
                .unwrap()
                .is_none()
        );

        let project_b = ProjectId::parse("project-b").unwrap();
        let other = store
            .capture(
                &CaptureRequest {
                    project_id: Some(&project_b),
                    worker_id: "worker-b",
                },
                8,
            )
            .unwrap()
            .unwrap();
        assert_eq!(other.work.id(), independent.id());
        store.release_capture(&first).unwrap();
        let recaptured = store
            .capture(
                &CaptureRequest {
                    project_id: Some(&project_a),
                    worker_id: "worker-c",
                },
                9,
            )
            .unwrap()
            .unwrap();
        assert_eq!(recaptured.work.id(), blocker.id());
    }

    #[test]
    fn relations_cannot_cross_projects() {
        let (mut store, _dir) = store();
        let a = scoped_sample("work-a", "project-a", None);
        let b = scoped_sample("work-b", "project-b", None);
        store.put(&a, WorkEvent::created(&a)).unwrap();
        store.put(&b, WorkEvent::created(&b)).unwrap();
        let relation =
            WorkRelation::new(a.id().clone(), b.id().clone(), RelationKind::Follows).unwrap();
        assert!(matches!(
            store.add_relation(&relation),
            Err(AppError::Conflict(_))
        ));
    }

    #[test]
    fn persisted_project_and_repository_context_are_immutable() {
        let (mut store, _dir) = store();
        let original = scoped_sample("work-a", "project-a", None);
        store.put(&original, WorkEvent::created(&original)).unwrap();
        let replacement = scoped_sample("work-a", "project-b", None);
        assert!(matches!(
            store.put(&replacement, WorkEvent::created(&replacement)),
            Err(AppError::Conflict(_))
        ));
        let loaded = store.get(original.id()).unwrap().unwrap();
        assert_eq!(loaded.attributes().project_id().as_str(), "project-a");
        assert_eq!(loaded.attributes().repository(), Some("repo:project-a"));
    }

    #[test]
    fn captured_start_consumes_exact_generation_and_direct_start_cannot_bypass_it() {
        let (mut store, _dir) = store();
        let work = sample();
        store.put(&work, WorkEvent::created(&work)).unwrap();
        let lease = store
            .capture(
                &CaptureRequest {
                    project_id: None,
                    worker_id: "queue-worker",
                },
                8,
            )
            .unwrap()
            .unwrap();
        let spec = execution_spec(&work);
        let execution = ExecutionId::parse("execution-capture").unwrap();
        let attempt = AttemptId::parse("attempt-capture").unwrap();
        let mut running = work.clone();
        running.start().unwrap();
        let direct = store.claim_attempt(&AttemptClaim {
            execution_id: &execution,
            attempt_id: &attempt,
            spec: &spec,
            work: &running,
            event: WorkEvent::started(&running, WorkStatus::Ready, 9),
            started_at_unix_ms: 9,
            capture: None,
        });
        assert!(matches!(direct, Err(AppError::Conflict(_))));

        store
            .claim_attempt(&AttemptClaim {
                execution_id: &execution,
                attempt_id: &attempt,
                spec: &spec,
                work: &running,
                event: WorkEvent::started(&running, WorkStatus::Ready, 9),
                started_at_unix_ms: 9,
                capture: Some(&lease),
            })
            .unwrap();
        let repeat = store.release_capture(&lease);
        assert!(matches!(repeat, Err(AppError::Conflict(_))));
    }

    #[test]
    fn daemon_recovery_releases_abandoned_captures() {
        let (mut store, _dir) = store();
        let work = sample();
        store.put(&work, WorkEvent::created(&work)).unwrap();
        store.configure_quota("queue:global", 1, None).unwrap();
        let first = store
            .capture(
                &CaptureRequest {
                    project_id: None,
                    worker_id: "lost-worker",
                },
                8,
            )
            .unwrap()
            .unwrap();
        assert_eq!(store.reclaim_captures().unwrap(), 1);
        let second = store
            .capture(
                &CaptureRequest {
                    project_id: None,
                    worker_id: "replacement-worker",
                },
                9,
            )
            .unwrap()
            .unwrap();
        assert_eq!(first.work.id(), second.work.id());
    }

    #[test]
    fn centralized_quota_uses_atomic_leases_and_generation_cas() {
        let (mut store, _dir) = store();
        assert_eq!(store.configure_quota("model", 3, None).unwrap(), 0);
        let first = store.acquire_quota("model", "project-a", 2).unwrap();
        assert!(matches!(
            store.acquire_quota("model", "project-b", 2),
            Err(AppError::Conflict(_))
        ));
        assert!(matches!(
            store.configure_quota("model", 4, Some(7)),
            Err(AppError::Conflict(_))
        ));
        assert_eq!(store.configure_quota("model", 4, Some(0)).unwrap(), 1);
        store.release_quota(&first).unwrap();
        assert!(store.acquire_quota("model", "project-b", 4).is_ok());
    }

    #[test]
    fn project_quota_exhaustion_does_not_block_an_unrelated_project() {
        let (mut store, _dir) = store();
        store.configure_quota("project:project-a", 0, None).unwrap();
        store.configure_quota("project:project-b", 1, None).unwrap();
        let a = scoped_sample("work-a", "project-a", None);
        let b = scoped_sample("work-b", "project-b", None);
        store.put(&a, WorkEvent::created(&a)).unwrap();
        store.put(&b, WorkEvent::created(&b)).unwrap();
        let lease = store
            .capture(
                &CaptureRequest {
                    project_id: None,
                    worker_id: "worker",
                },
                8,
            )
            .unwrap()
            .unwrap();
        assert_eq!(lease.work.id(), b.id());
    }

    #[test]
    fn inbound_receipt_is_idempotent_and_preserves_project_context() {
        let (mut store, _dir) = store();
        let work = scoped_sample("inbound-1", "project-a", Some("operator-a"));
        assert!(
            store
                .put_inbound("source", "record-1", &work, WorkEvent::created(&work))
                .unwrap()
        );
        let duplicate = scoped_sample("inbound-2", "project-a", Some("operator-a"));
        assert!(
            !store
                .put_inbound(
                    "source",
                    "record-1",
                    &duplicate,
                    WorkEvent::created(&duplicate),
                )
                .unwrap()
        );
        assert_eq!(store.list().unwrap().len(), 1);
        let loaded = store.get(work.id()).unwrap().unwrap();
        assert_eq!(loaded.attributes().project_id().as_str(), "project-a");
        assert_eq!(loaded.attributes().repository(), Some("repo:project-a"));
    }

    #[test]
    fn external_input_park_enqueues_targeted_best_effort_publication() {
        let (mut store, _dir) = store();
        let work = scoped_sample("notified", "project-a", Some("operator-a"));
        store.put(&work, WorkEvent::created(&work)).unwrap();
        let spec = execution_spec(&work);
        let execution = ExecutionId::parse("execution-notify").unwrap();
        let attempt = AttemptId::parse("attempt-notify").unwrap();
        let mut running = work.clone();
        running.start().unwrap();
        store
            .claim_attempt(&AttemptClaim {
                execution_id: &execution,
                attempt_id: &attempt,
                spec: &spec,
                work: &running,
                event: WorkEvent::started(&running, WorkStatus::Ready, 8),
                started_at_unix_ms: 8,
                capture: None,
            })
            .unwrap();
        store
            .record_checkpoint(work.id(), &execution, &attempt, 9)
            .unwrap();
        let mut parked = running;
        parked.park().unwrap();
        store
            .park_attempt_with_checkpoint(
                &parked,
                WorkEvent::parked(&parked, WorkStatus::Running, 9),
                &execution,
                &attempt,
                true,
                None,
            )
            .unwrap();

        let pending = store.pending_publications(20).unwrap();
        let notification = pending
            .iter()
            .find(|item| item.kind == PublicationKind::ExternalInputRequired)
            .unwrap();
        assert_eq!(notification.target, "operator-a");
        assert_eq!(notification.project_id.as_str(), "project-a");
        store
            .record_publication_attempt(
                notification.publication_id,
                false,
                10,
                Some("secret remote error"),
            )
            .unwrap();
        assert_eq!(
            store.get(work.id()).unwrap().unwrap().status(),
            WorkStatus::Parked
        );
    }

    #[cfg(unix)]
    #[test]
    fn store_database_does_not_follow_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside.sqlite");
        std::fs::write(&outside, "protected").unwrap();
        std::os::unix::fs::symlink(&outside, dir.path().join("workengine.sqlite")).unwrap();
        assert!(SqliteStore::open(dir.path()).is_err());
        assert_eq!(std::fs::read_to_string(outside).unwrap(), "protected");
    }
}
