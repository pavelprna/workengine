//! SQLite WorkStore. Status and event append are one transaction.

use std::path::Path;
use std::str::FromStr;

use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use workengine_application::{AppError, AttemptClaim, SequencedEvent, WorkQuery, WorkStore};
use workengine_domain::{
    AttemptId, ConfirmedOutcome, EVENT_SCHEMA_VERSION, EventKind, ExecutionId, ExecutionSpec,
    OutcomeKind, Work, WorkAttributes, WorkEvent, WorkId, WorkStatus,
};

const WORK_SCHEMA_VERSION: u32 = 1;
/// SQLite `user_version`. Distinct from per-row `schema_version` on Work.
const STORE_USER_VERSION: i32 = 3;

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
    runtime_digest: &'a str,
    wall_clock_budget_ms: u64,
    retry_limit: u32,
    channel_policy: &'a str,
    secret_refs: Vec<SecretRefPayload<'a>>,
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
        std::fs::create_dir_all(data_dir).map_err(AppError::store)?;
        restrict_directory(data_dir)?;
        let database = data_dir.join("workengine.sqlite");
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
            WorkAttributes::new(row.goal, row.worker_profile)?,
            row.workspace_root,
            row.created_at_unix_ms as u64,
        ))
    }
}

impl SqliteObserver {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, AppError> {
        let database = data_dir.as_ref().join("workengine.sqlite");
        let conn = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(AppError::store)?;
        conn.execute_batch("PRAGMA query_only = ON; PRAGMA foreign_keys = ON;")
            .map_err(AppError::store)?;
        Ok(Self { conn })
    }
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
}

fn get_connection(conn: &Connection, id: &WorkId) -> Result<Option<Work>, AppError> {
    conn
        .query_row(
            "SELECT id, status, goal, worker_profile, workspace_root, created_at_unix_ms, schema_version
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
            "SELECT id, status, goal, worker_profile, workspace_root, created_at_unix_ms, schema_version
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
        }
    };
}

impl_work_query!(SqliteStore);
impl_work_query!(SqliteObserver);

impl WorkStore for SqliteStore {
    fn put(&mut self, work: &Work, event: WorkEvent) -> Result<(), AppError> {
        let tx = self.conn.transaction().map_err(AppError::store)?;
        put_in_tx(&tx, work, &event)?;
        tx.commit().map_err(AppError::store)?;
        Ok(())
    }

    fn claim_attempt(&mut self, claim: &AttemptClaim<'_>) -> Result<(), AppError> {
        let tx = self.conn.transaction().map_err(AppError::store)?;
        let spec_payload = encode_execution_spec(claim.spec)?;
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
        tx.execute(
            "INSERT INTO attempts (id, execution_id, state, created_at_unix_ms)
             VALUES (?1, ?2, 'active', ?3)",
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
                "UPDATE attempts SET state = 'retried', terminal_reason = 'channel_retry'
                 WHERE id = ?1 AND execution_id = ?2 AND state = 'active'",
                params![previous_attempt_id.as_str(), execution_id.as_str()],
            )
            .map_err(AppError::store)?;
        if changed != 1 {
            return Err(attempt_conflict(previous_attempt_id));
        }
        tx.execute(
            "INSERT INTO attempts (id, execution_id, state, created_at_unix_ms)
             VALUES (?1, ?2, 'active', ?3)",
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
        let changed = tx
            .execute(
                "UPDATE attempts SET state = 'confirmed', terminal_reason = ?1
                 WHERE id = ?2 AND execution_id = ?3 AND state = 'active'",
                params![
                    outcome.outcome().kind().as_str(),
                    outcome.attempt_id().as_str(),
                    outcome.execution_id().as_str(),
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
        let changed = tx
            .execute(
                "UPDATE attempts SET state = 'parked', terminal_reason = 'parked'
                 WHERE id = ?1 AND execution_id = ?2 AND state = 'active'",
                params![attempt_id.as_str(), execution_id.as_str()],
            )
            .map_err(AppError::store)?;
        if changed != 1 {
            return Err(attempt_conflict(attempt_id));
        }
        insert_event(&tx, &event)?;
        tx.commit().map_err(AppError::store)
    }
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
        conn.pragma_update(None, "user_version", STORE_USER_VERSION)
            .map_err(AppError::store)?;
    } else if current == 2 {
        conn.execute_batch(MIGRATION_3).map_err(AppError::store)?;
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
        "INSERT INTO works (id, status, goal, worker_profile, workspace_root, created_at_unix_ms, schema_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
            status = excluded.status,
            goal = excluded.goal,
            worker_profile = excluded.worker_profile,
            workspace_root = excluded.workspace_root
         WHERE works.active_attempt_id IS NULL",
        params![
            work.id().as_str(),
            work.status().as_str(),
            work.attributes().goal(),
            work.attributes().worker_profile(),
            work.workspace_root(),
            work.created_at_unix_ms() as i64,
            WORK_SCHEMA_VERSION as i64,
        ],
    )
    .map_err(AppError::store)
    .and_then(|changed| {
        if changed == 1 {
            Ok(())
        } else {
            Err(AppError::Conflict(format!(
                "Work {} has an active attempt",
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
        (Some(goal), Some(profile)) => Some(WorkAttributes::new(goal, profile)?),
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
        OUTCOME_SCHEMA_VERSION, Outcome, OutcomeKind, replay,
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

    fn execution_spec(work: &Work) -> ExecutionSpec {
        ExecutionSpec::new(
            EXECUTION_SPEC_SCHEMA_VERSION,
            work.id().clone(),
            work.attributes().worker_profile(),
            ContentDigest::parse(format!("sha256:{}", "1".repeat(64))).unwrap(),
            ContentDigest::parse(format!("sha256:{}", "2".repeat(64))).unwrap(),
            5_000,
            0,
            ChannelPolicy::Fail,
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
    fn unknown_store_user_version_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workengine.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 4).unwrap();
        drop(conn);
        let err = match SqliteStore::open(dir.path()) {
            Ok(_) => panic!("expected unknown store version to fail"),
            Err(err) => err,
        };
        assert!(matches!(
            err,
            AppError::Domain(workengine_domain::DomainError::UnsupportedSchemaVersion(4))
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
            })
            .unwrap();

        let second = store.claim_attempt(&AttemptClaim {
            execution_id: &ExecutionId::parse("execution-2").unwrap(),
            attempt_id: &AttemptId::parse("attempt-2").unwrap(),
            spec: &spec,
            work: &running,
            event: WorkEvent::started(&running, WorkStatus::Ready, 9),
            started_at_unix_ms: 9,
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
}
