use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use workengine_domain::{
    OUTCOME_SCHEMA_VERSION, Outcome, OutcomeKind, Work, WorkEvent, WorkId, WorkStatus,
};

use crate::clock::Clock;
use crate::error::AppError;
use crate::ports::{RunRequest, WorkStore, WorkerRunner, WorkspaceFactory};
use crate::{complete, create, next, park, recover_unconfirmed, start};

const BUDGET: Duration = Duration::from_secs(5);

struct FakeClock {
    unix_ms: u64,
}

impl Clock for FakeClock {
    fn unix_ms(&self) -> u64 {
        self.unix_ms
    }
}

#[derive(Default)]
struct FakeStore {
    works: HashMap<String, Work>,
    events: Vec<WorkEvent>,
}

impl WorkStore for FakeStore {
    fn get(&self, id: &WorkId) -> Result<Option<Work>, AppError> {
        Ok(self.works.get(id.as_str()).cloned())
    }

    fn list(&self) -> Result<Vec<Work>, AppError> {
        Ok(self.works.values().cloned().collect())
    }

    fn events(&self, id: &WorkId) -> Result<Vec<WorkEvent>, AppError> {
        Ok(self
            .events
            .iter()
            .filter(|e| e.work_id() == id)
            .cloned()
            .collect())
    }

    fn put(&mut self, work: &Work, event: WorkEvent) -> Result<(), AppError> {
        self.works
            .insert(work.id().as_str().to_owned(), work.clone());
        self.events.push(event);
        Ok(())
    }
}

#[derive(Default)]
struct FakeWorkspace {
    artifacts: HashMap<String, Vec<u8>>,
    memory: RefCell<HashMap<String, Vec<(WorkStatus, OutcomeKind)>>>,
}

impl WorkspaceFactory for FakeWorkspace {
    fn bind(&self, work_id: &WorkId) -> Result<PathBuf, AppError> {
        Ok(PathBuf::from(format!("/workspace/{work_id}")))
    }

    fn read_artifact(&self, work_id: &WorkId) -> Result<Option<Vec<u8>>, AppError> {
        Ok(self.artifacts.get(work_id.as_str()).cloned())
    }

    fn record_memory(
        &self,
        work_id: &WorkId,
        status: WorkStatus,
        outcome_kind: OutcomeKind,
    ) -> Result<(), AppError> {
        let mut memory = self.memory.borrow_mut();
        let lines = memory.entry(work_id.as_str().to_owned()).or_default();
        if lines.last() == Some(&(status, outcome_kind)) {
            return Ok(());
        }
        lines.push((status, outcome_kind));
        Ok(())
    }
}

struct FakeRunner {
    runs: Cell<u32>,
    kind: OutcomeKind,
}

impl FakeRunner {
    fn new(kind: OutcomeKind) -> Self {
        Self {
            runs: Cell::new(0),
            kind,
        }
    }
}

impl WorkerRunner for FakeRunner {
    fn run(&self, request: &RunRequest<'_>) -> Result<Outcome, AppError> {
        self.runs.set(self.runs.get() + 1);
        Outcome::new(
            OUTCOME_SCHEMA_VERSION,
            self.kind,
            request.work.attributes().worker_profile(),
        )
        .map_err(AppError::from)
    }

    fn decode(&self, bytes: &[u8]) -> Result<Outcome, AppError> {
        let raw = std::str::from_utf8(bytes).map_err(AppError::outcome_schema)?;
        let kind: OutcomeKind = raw.trim().parse().map_err(AppError::outcome_schema)?;
        Outcome::new(OUTCOME_SCHEMA_VERSION, kind, "stub").map_err(AppError::from)
    }
}

fn ready_work(store: &mut FakeStore, clock: &FakeClock) -> Work {
    create(store, clock, "do the thing", "stub").unwrap()
}

fn start_work(
    store: &mut FakeStore,
    ws: &FakeWorkspace,
    runner: &impl WorkerRunner,
    clock: &FakeClock,
    id: &WorkId,
) -> Result<Work, AppError> {
    start(store, ws, runner, clock, id, BUDGET)
}

#[test]
fn create_then_next_returns_it() {
    let mut store = FakeStore::default();
    let clock = FakeClock { unix_ms: 10 };
    let work = ready_work(&mut store, &clock);
    assert_eq!(work.status(), WorkStatus::Ready);
    assert_eq!(next(&store).unwrap().as_ref(), Some(work.id()));
}

#[test]
fn next_fifo_oldest_ready() {
    let mut store = FakeStore::default();
    let first = create(&mut store, &FakeClock { unix_ms: 1 }, "a", "stub").unwrap();
    let _second = create(&mut store, &FakeClock { unix_ms: 2 }, "b", "stub").unwrap();
    assert_eq!(next(&store).unwrap().as_ref(), Some(first.id()));
}

#[test]
fn next_prefers_ready_over_parked() {
    let mut store = FakeStore::default();
    let clock = FakeClock { unix_ms: 1 };
    let parked = create(&mut store, &clock, "old", "stub").unwrap();
    let mut parked_work = store.get(parked.id()).unwrap().unwrap();
    parked_work.start().unwrap();
    store
        .put(
            &parked_work,
            workengine_domain::WorkEvent::started(&parked_work, WorkStatus::Ready, 2),
        )
        .unwrap();
    park(&mut store, &clock, parked.id()).unwrap();
    let ready = create(&mut store, &FakeClock { unix_ms: 99 }, "new", "stub").unwrap();
    assert_eq!(next(&store).unwrap().as_ref(), Some(ready.id()));
}

#[test]
fn next_is_idempotent() {
    let mut store = FakeStore::default();
    let work = ready_work(&mut store, &FakeClock { unix_ms: 1 });
    assert_eq!(next(&store).unwrap(), next(&store).unwrap());
    assert_eq!(next(&store).unwrap().as_ref(), Some(work.id()));
}

#[test]
fn start_spawns_once_and_completes() {
    let mut store = FakeStore::default();
    let ws = FakeWorkspace::default();
    let runner = FakeRunner::new(OutcomeKind::Succeeded);
    let clock = FakeClock { unix_ms: 1 };
    let work = ready_work(&mut store, &clock);
    let done = start_work(&mut store, &ws, &runner, &clock, work.id()).unwrap();
    assert_eq!(done.status(), WorkStatus::Succeeded);
    assert_eq!(runner.runs.get(), 1);
    assert_eq!(ws.memory.borrow()[work.id().as_str()].len(), 1);
    let events = store.events(work.id()).unwrap();
    assert_eq!(events.len(), 3); // created, started, completed
}

#[test]
fn start_with_artifact_does_not_spawn() {
    let mut store = FakeStore::default();
    let clock = FakeClock { unix_ms: 1 };
    let work = ready_work(&mut store, &clock);
    let mut ws = FakeWorkspace::default();
    ws.artifacts
        .insert(work.id().as_str().to_owned(), b"succeeded".to_vec());
    let runner = FakeRunner::new(OutcomeKind::Failed);
    let done = start_work(&mut store, &ws, &runner, &clock, work.id()).unwrap();
    assert_eq!(done.status(), WorkStatus::Succeeded);
    assert_eq!(runner.runs.get(), 0);
}

#[test]
fn complete_is_idempotent_and_does_not_append_a_second_event() {
    let mut store = FakeStore::default();
    let ws = FakeWorkspace::default();
    let runner = FakeRunner::new(OutcomeKind::Succeeded);
    let clock = FakeClock { unix_ms: 1 };
    let work = ready_work(&mut store, &clock);
    start_work(&mut store, &ws, &runner, &clock, work.id()).unwrap();
    let n = store.events(work.id()).unwrap().len();
    let outcome = Outcome::new(OUTCOME_SCHEMA_VERSION, OutcomeKind::Succeeded, "stub").unwrap();
    complete(&mut store, &ws, &clock, work.id(), &outcome).unwrap();
    assert_eq!(store.events(work.id()).unwrap().len(), n);
    assert_eq!(ws.memory.borrow()[work.id().as_str()].len(), 1);
}

#[test]
fn complete_retry_writes_memory_if_the_first_write_was_lost() {
    let mut store = FakeStore::default();
    let ws = FakeWorkspace::default();
    let runner = FakeRunner::new(OutcomeKind::Succeeded);
    let clock = FakeClock { unix_ms: 1 };
    let work = ready_work(&mut store, &clock);
    start_work(&mut store, &ws, &runner, &clock, work.id()).unwrap();
    ws.memory.borrow_mut().clear();
    let outcome = Outcome::new(OUTCOME_SCHEMA_VERSION, OutcomeKind::Succeeded, "stub").unwrap();
    complete(&mut store, &ws, &clock, work.id(), &outcome).unwrap();
    assert_eq!(ws.memory.borrow()[work.id().as_str()].len(), 1);
    complete(&mut store, &ws, &clock, work.id(), &outcome).unwrap();
    assert_eq!(ws.memory.borrow()[work.id().as_str()].len(), 1);
}

#[test]
fn recover_parks_leftover_running() {
    let mut store = FakeStore::default();
    let clock = FakeClock { unix_ms: 1 };
    let work = ready_work(&mut store, &clock);
    let mut running = store.get(work.id()).unwrap().unwrap();
    running.start().unwrap();
    store
        .put(
            &running,
            workengine_domain::WorkEvent::started(&running, WorkStatus::Ready, 2),
        )
        .unwrap();
    let parked = recover_unconfirmed(&mut store, &clock).unwrap();
    assert_eq!(parked.len(), 1);
    assert_eq!(
        store.get(work.id()).unwrap().unwrap().status(),
        WorkStatus::Parked
    );
    assert_eq!(next(&store).unwrap().as_ref(), Some(work.id()));
}

#[test]
fn park_does_not_run_the_worker() {
    let mut store = FakeStore::default();
    let runner = FakeRunner::new(OutcomeKind::Succeeded);
    let clock = FakeClock { unix_ms: 1 };
    let work = ready_work(&mut store, &clock);
    let mut running = store.get(work.id()).unwrap().unwrap();
    running.start().unwrap();
    store
        .put(
            &running,
            workengine_domain::WorkEvent::started(&running, WorkStatus::Ready, 2),
        )
        .unwrap();
    park(&mut store, &clock, work.id()).unwrap();
    park(&mut store, &clock, work.id()).unwrap();
    assert_eq!(runner.runs.get(), 0);
    assert_eq!(
        store.get(work.id()).unwrap().unwrap().status(),
        WorkStatus::Parked
    );
}

#[test]
fn start_after_success_is_illegal() {
    let mut store = FakeStore::default();
    let ws = FakeWorkspace::default();
    let runner = FakeRunner::new(OutcomeKind::Succeeded);
    let clock = FakeClock { unix_ms: 1 };
    let work = ready_work(&mut store, &clock);
    start_work(&mut store, &ws, &runner, &clock, work.id()).unwrap();
    let err = start_work(&mut store, &ws, &runner, &clock, work.id()).unwrap_err();
    assert!(matches!(err, AppError::Domain(_)));
}

#[test]
fn timeout_outcome_fails_work_without_a_second_spawn_on_complete() {
    let mut store = FakeStore::default();
    let ws = FakeWorkspace::default();
    let runner = FakeRunner::new(OutcomeKind::TimedOut);
    let clock = FakeClock { unix_ms: 1 };
    let work = ready_work(&mut store, &clock);
    let done = start_work(&mut store, &ws, &runner, &clock, work.id()).unwrap();
    assert_eq!(done.status(), WorkStatus::Failed);
    assert_eq!(runner.runs.get(), 1);
}

#[test]
fn next_skips_succeeded() {
    let mut store = FakeStore::default();
    let ws = FakeWorkspace::default();
    let runner = FakeRunner::new(OutcomeKind::Succeeded);
    let clock = FakeClock { unix_ms: 1 };
    let work = ready_work(&mut store, &clock);
    start_work(&mut store, &ws, &runner, &clock, work.id()).unwrap();
    assert_eq!(next(&store).unwrap(), None);
}

#[test]
fn start_on_parked_with_artifact_does_not_spawn_or_start_again() {
    let mut store = FakeStore::default();
    let clock = FakeClock { unix_ms: 1 };
    let work = ready_work(&mut store, &clock);
    let mut running = store.get(work.id()).unwrap().unwrap();
    running.start().unwrap();
    store
        .put(
            &running,
            workengine_domain::WorkEvent::started(&running, WorkStatus::Ready, 2),
        )
        .unwrap();
    park(&mut store, &clock, work.id()).unwrap();
    let mut ws = FakeWorkspace::default();
    ws.artifacts
        .insert(work.id().as_str().to_owned(), b"succeeded".to_vec());
    let runner = FakeRunner::new(OutcomeKind::Failed);
    let done = start_work(&mut store, &ws, &runner, &clock, work.id()).unwrap();
    assert_eq!(done.status(), WorkStatus::Succeeded);
    assert_eq!(runner.runs.get(), 0);
    let kinds: Vec<_> = store
        .events(work.id())
        .unwrap()
        .iter()
        .map(|e| e.kind())
        .collect();
    assert_eq!(
        kinds,
        vec![
            workengine_domain::EventKind::Created,
            workengine_domain::EventKind::Started,
            workengine_domain::EventKind::Parked,
            workengine_domain::EventKind::Completed,
        ]
    );
}

#[test]
fn complete_after_recover_applies_to_parked() {
    let mut store = FakeStore::default();
    let ws = FakeWorkspace::default();
    let clock = FakeClock { unix_ms: 1 };
    let work = ready_work(&mut store, &clock);
    let mut running = store.get(work.id()).unwrap().unwrap();
    running.start().unwrap();
    store
        .put(
            &running,
            workengine_domain::WorkEvent::started(&running, WorkStatus::Ready, 2),
        )
        .unwrap();
    recover_unconfirmed(&mut store, &clock).unwrap();
    assert_eq!(
        store.get(work.id()).unwrap().unwrap().status(),
        WorkStatus::Parked
    );
    let outcome = Outcome::new(OUTCOME_SCHEMA_VERSION, OutcomeKind::Succeeded, "stub").unwrap();
    let done = complete(&mut store, &ws, &clock, work.id(), &outcome).unwrap();
    assert_eq!(done.status(), WorkStatus::Succeeded);
}

#[test]
fn bad_artifact_is_schema_error_and_does_not_complete() {
    let mut store = FakeStore::default();
    let clock = FakeClock { unix_ms: 1 };
    let work = ready_work(&mut store, &clock);
    let mut ws = FakeWorkspace::default();
    ws.artifacts
        .insert(work.id().as_str().to_owned(), b"not-json".to_vec());
    let runner = FakeRunner::new(OutcomeKind::Succeeded);
    let err = start_work(&mut store, &ws, &runner, &clock, work.id()).unwrap_err();
    assert!(matches!(err, AppError::OutcomeSchema(_)));
    assert_eq!(
        store.get(work.id()).unwrap().unwrap().status(),
        WorkStatus::Ready
    );
    assert_eq!(runner.runs.get(), 0);
}

#[test]
fn started_and_completed_events_use_clock_not_work_birth() {
    let mut store = FakeStore::default();
    let ws = FakeWorkspace::default();
    let runner = FakeRunner::new(OutcomeKind::Succeeded);
    let work = ready_work(&mut store, &FakeClock { unix_ms: 10 });
    start_work(
        &mut store,
        &ws,
        &runner,
        &FakeClock { unix_ms: 50 },
        work.id(),
    )
    .unwrap();
    let events = store.events(work.id()).unwrap();
    assert_eq!(events[0].created_at_unix_ms(), 10);
    assert_eq!(events[1].created_at_unix_ms(), 50);
    assert_eq!(events[2].created_at_unix_ms(), 50);
}

struct BoomRunner;

impl WorkerRunner for BoomRunner {
    fn run(&self, _request: &RunRequest<'_>) -> Result<Outcome, AppError> {
        Err(AppError::worker("spawn failed"))
    }

    fn decode(&self, _bytes: &[u8]) -> Result<Outcome, AppError> {
        Err(AppError::worker("unused"))
    }
}

#[test]
fn runner_error_completes_as_channel_error_without_returning_err() {
    let mut store = FakeStore::default();
    let ws = FakeWorkspace::default();
    let clock = FakeClock { unix_ms: 1 };
    let work = ready_work(&mut store, &clock);
    let done = start_work(&mut store, &ws, &BoomRunner, &clock, work.id()).unwrap();
    assert_eq!(done.status(), WorkStatus::Failed);
    let last = store.events(work.id()).unwrap().pop().unwrap();
    assert_eq!(last.outcome_kind(), Some(OutcomeKind::ChannelError));
}
