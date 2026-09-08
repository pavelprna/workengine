//! SQLite WorkStore. Status and event append are one transaction.

use std::path::Path;
use std::str::FromStr;

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use workengine_application::{AppError, WorkStore};
use workengine_domain::{
    EVENT_SCHEMA_VERSION, EventKind, OutcomeKind, Work, WorkAttributes, WorkEvent, WorkId,
    WorkStatus,
};

const WORK_SCHEMA_VERSION: u32 = 1;

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

pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, AppError> {
        let data_dir = data_dir.as_ref();
        std::fs::create_dir_all(data_dir).map_err(AppError::store)?;
        let conn = Connection::open(data_dir.join("workengine.sqlite")).map_err(AppError::store)?;
        conn.execute_batch(
            "
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
                work_id TEXT NOT NULL,
                payload TEXT NOT NULL
            );
            ",
        )
        .map_err(AppError::store)?;
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

struct WorkRow {
    id: String,
    status: String,
    goal: String,
    worker_profile: String,
    workspace_root: Option<String>,
    created_at_unix_ms: i64,
    schema_version: i64,
}

impl WorkStore for SqliteStore {
    fn get(&self, id: &WorkId) -> Result<Option<Work>, AppError> {
        self.conn
            .query_row(
                "SELECT id, status, goal, worker_profile, workspace_root, created_at_unix_ms, schema_version
                 FROM works WHERE id = ?1",
                [id.as_str()],
                Self::read_row,
            )
            .optional()
            .map_err(AppError::store)?
            .map(Self::work_from_row)
            .transpose()
    }

    fn list(&self) -> Result<Vec<Work>, AppError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, status, goal, worker_profile, workspace_root, created_at_unix_ms, schema_version
                 FROM works",
            )
            .map_err(AppError::store)?;
        let rows = stmt
            .query_map([], Self::read_row)
            .map_err(AppError::store)?;
        let mut works = Vec::new();
        for row in rows {
            works.push(Self::work_from_row(row.map_err(AppError::store)?)?);
        }
        Ok(works)
    }

    fn events(&self, id: &WorkId) -> Result<Vec<WorkEvent>, AppError> {
        let mut stmt = self
            .conn
            .prepare("SELECT payload FROM events WHERE work_id = ?1 ORDER BY seq")
            .map_err(AppError::store)?;
        let rows = stmt
            .query_map([id.as_str()], |row| row.get::<_, String>(0))
            .map_err(AppError::store)?;
        let mut events = Vec::new();
        for row in rows {
            let payload = row.map_err(AppError::store)?;
            events.push(decode_event(&payload)?);
        }
        Ok(events)
    }

    fn put(&mut self, work: &Work, event: WorkEvent) -> Result<(), AppError> {
        let tx = self.conn.transaction().map_err(AppError::store)?;
        put_in_tx(&tx, work, &event)?;
        tx.commit().map_err(AppError::store)?;
        Ok(())
    }
}

fn put_in_tx(tx: &Transaction<'_>, work: &Work, event: &WorkEvent) -> Result<(), AppError> {
    tx.execute(
        "INSERT INTO works (id, status, goal, worker_profile, workspace_root, created_at_unix_ms, schema_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
            status = excluded.status,
            goal = excluded.goal,
            worker_profile = excluded.worker_profile,
            workspace_root = excluded.workspace_root",
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
    .map_err(AppError::store)?;
    let payload = encode_event(event)?;
    tx.execute(
        "INSERT INTO events (work_id, payload) VALUES (?1, ?2)",
        params![work.id().as_str(), payload],
    )
    .map_err(AppError::store)?;
    Ok(())
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
    use workengine_domain::{OUTCOME_SCHEMA_VERSION, Outcome, OutcomeKind, replay};

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
            .put(&running, WorkEvent::started(&running, from))
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
        work.bind_workspace("/ws/work-1");
        store.put(&work, WorkEvent::started(&work, from)).unwrap();
        let outcome = Outcome::new(OUTCOME_SCHEMA_VERSION, OutcomeKind::Succeeded, "stub").unwrap();
        let from = work.status();
        work.complete(&outcome).unwrap();
        store
            .put(
                &work,
                WorkEvent::completed(&work, from, OutcomeKind::Succeeded),
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
}
