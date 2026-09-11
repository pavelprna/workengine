use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use workengine_domain::{
    AttemptId, ChannelPolicy, ConfirmedOutcome, ContentDigest, EXECUTION_SPEC_SCHEMA_VERSION,
    ExecutionId, ExecutionSpec, OUTCOME_SCHEMA_VERSION, Outcome, OutcomeKind, ProjectId,
    RuntimeKind, Work, WorkEvent, WorkId, WorkStatus,
};

use crate::clock::Clock;
use crate::error::{AppError, ChannelReaction};
use crate::ports::{
    AttemptClaim, AttemptRecorder, BindRequest, ExecutionObservation, ExpectedContext,
    InboundRecord, InboundSignal, InboundSource, InboundStore, MutationRequest, MutationResult,
    ProcessEvent, Publication, PublicationKind, PublicationStore, Publisher, RemoteMutation,
    RunRequest, SequencedEvent, StartRequest, WorkQuery, WorkStore, WorkerExit, WorkerRunner,
    WorkspaceFactory,
};
use crate::{
    complete, create, dispatch_publications, mutate_remote, next, park, poll_inbound,
    recover_unconfirmed, start,
};

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
    active: HashMap<String, (String, String)>,
    inbound_receipts: HashSet<(String, String)>,
}

impl WorkQuery for FakeStore {
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

    fn events_after(
        &self,
        after_seq: u64,
        work_id: Option<&WorkId>,
    ) -> Result<Vec<SequencedEvent>, AppError> {
        Ok(self
            .events
            .iter()
            .enumerate()
            .filter(|(index, event)| {
                (*index as u64) + 1 > after_seq && work_id.is_none_or(|id| event.work_id() == id)
            })
            .map(|(index, event)| SequencedEvent {
                seq: index as u64 + 1,
                event: event.clone(),
            })
            .collect())
    }

    fn executions(&self, _id: &WorkId) -> Result<Vec<ExecutionObservation>, AppError> {
        Ok(Vec::new())
    }
}

impl AttemptRecorder for FakeStore {
    fn heartbeat_attempt(
        &mut self,
        _work_id: &WorkId,
        _execution_id: &ExecutionId,
        _attempt_id: &AttemptId,
        _observed_at_unix_ms: u64,
    ) -> Result<(), AppError> {
        Ok(())
    }

    fn record_process_event(
        &mut self,
        _work_id: &WorkId,
        _execution_id: &ExecutionId,
        _attempt_id: &AttemptId,
        _event: ProcessEvent,
        _observed_at_unix_ms: u64,
    ) -> Result<(), AppError> {
        Ok(())
    }
}

impl WorkStore for FakeStore {
    fn put(&mut self, work: &Work, event: WorkEvent) -> Result<(), AppError> {
        self.works
            .insert(work.id().as_str().to_owned(), work.clone());
        self.events.push(event);
        Ok(())
    }

    fn claim_attempt(&mut self, claim: &AttemptClaim<'_>) -> Result<(), AppError> {
        let key = claim.work.id().as_str().to_owned();
        if self.active.contains_key(&key)
            || self
                .works
                .get(&key)
                .is_none_or(|stored| stored.status() == WorkStatus::Running)
        {
            return Err(AppError::Conflict("active attempt".to_owned()));
        }
        self.active.insert(
            key.clone(),
            (
                claim.execution_id.as_str().to_owned(),
                claim.attempt_id.as_str().to_owned(),
            ),
        );
        self.works.insert(key, claim.work.clone());
        self.events.push(claim.event.clone());
        Ok(())
    }

    fn retry_attempt(
        &mut self,
        execution_id: &ExecutionId,
        previous_attempt_id: &AttemptId,
        next_attempt_id: &AttemptId,
        _started_at_unix_ms: u64,
    ) -> Result<(), AppError> {
        let Some((_, active_attempt)) = self
            .active
            .values_mut()
            .find(|(execution, _)| execution == execution_id.as_str())
        else {
            return Err(AppError::Conflict("missing execution lease".to_owned()));
        };
        if active_attempt != previous_attempt_id.as_str() {
            return Err(AppError::Conflict("foreign attempt".to_owned()));
        }
        *active_attempt = next_attempt_id.as_str().to_owned();
        Ok(())
    }

    fn confirm_attempt(
        &mut self,
        work: &Work,
        event: WorkEvent,
        outcome: &ConfirmedOutcome,
    ) -> Result<(), AppError> {
        self.release_matching(work.id(), outcome.execution_id(), outcome.attempt_id())?;
        self.works
            .insert(work.id().as_str().to_owned(), work.clone());
        self.events.push(event);
        Ok(())
    }

    fn park_attempt(
        &mut self,
        work: &Work,
        event: WorkEvent,
        execution_id: &ExecutionId,
        attempt_id: &AttemptId,
    ) -> Result<(), AppError> {
        self.release_matching(work.id(), execution_id, attempt_id)?;
        self.works
            .insert(work.id().as_str().to_owned(), work.clone());
        self.events.push(event);
        Ok(())
    }
}

impl FakeStore {
    fn release_matching(
        &mut self,
        work_id: &WorkId,
        execution_id: &ExecutionId,
        attempt_id: &AttemptId,
    ) -> Result<(), AppError> {
        let key = work_id.as_str();
        let expected = (
            execution_id.as_str().to_owned(),
            attempt_id.as_str().to_owned(),
        );
        if self.active.get(key) != Some(&expected) {
            return Err(AppError::Conflict("foreign attempt".to_owned()));
        }
        self.active.remove(key);
        Ok(())
    }
}

impl InboundStore for FakeStore {
    fn put_inbound(
        &mut self,
        source_id: &str,
        record_id: &str,
        work: &Work,
        event: WorkEvent,
    ) -> Result<bool, AppError> {
        if !self
            .inbound_receipts
            .insert((source_id.to_owned(), record_id.to_owned()))
        {
            return Ok(false);
        }
        self.put(work, event)?;
        Ok(true)
    }
}

#[derive(Default)]
struct FakeWorkspace {
    artifacts: HashMap<String, Vec<u8>>,
    memory: RefCell<HashMap<String, Vec<(WorkStatus, OutcomeKind)>>>,
    goals: RefCell<HashMap<String, String>>,
}

impl WorkspaceFactory for FakeWorkspace {
    fn bind(&self, request: &BindRequest<'_>) -> Result<PathBuf, AppError> {
        self.goals
            .borrow_mut()
            .insert(request.work_id.as_str().to_owned(), request.goal.to_owned());
        Ok(PathBuf::from(format!("/workspace/{}", request.work_id)))
    }

    fn read_artifact(
        &self,
        work_id: &WorkId,
        _project_id: &workengine_domain::ProjectId,
    ) -> Result<Option<Vec<u8>>, AppError> {
        Ok(self.artifacts.get(work_id.as_str()).cloned())
    }

    fn record_memory(
        &self,
        work_id: &WorkId,
        _project_id: &workengine_domain::ProjectId,
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
    fn run(&self, request: &mut RunRequest<'_>) -> Result<WorkerExit, AppError> {
        self.runs.set(self.runs.get() + 1);
        Outcome::new(
            OUTCOME_SCHEMA_VERSION,
            self.kind,
            request.work.attributes().worker_profile(),
        )
        .map(WorkerExit::Completed)
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
    start_work_with(store, ws, runner, clock, id, 0)
}

fn start_work_with(
    store: &mut FakeStore,
    ws: &FakeWorkspace,
    runner: &impl WorkerRunner,
    clock: &FakeClock,
    id: &WorkId,
    retry_limit: u32,
) -> Result<Work, AppError> {
    let spec = execution_spec(id, retry_limit);
    start(
        store,
        ws,
        runner,
        clock,
        &StartRequest {
            id,
            execution_spec: &spec,
            checkout: None,
            capture: None,
        },
    )
}

fn execution_spec(id: &WorkId, retry_limit: u32) -> ExecutionSpec {
    ExecutionSpec::new(
        EXECUTION_SPEC_SCHEMA_VERSION,
        id.clone(),
        "stub",
        ContentDigest::parse(format!("sha256:{}", "1".repeat(64))).unwrap(),
        RuntimeKind::Stub,
        ContentDigest::parse(format!("sha256:{}", "2".repeat(64))).unwrap(),
        BUDGET.as_millis() as u64,
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
fn ready_work_rejects_a_preexisting_outcome_artifact() {
    let mut store = FakeStore::default();
    let clock = FakeClock { unix_ms: 1 };
    let work = ready_work(&mut store, &clock);
    let mut ws = FakeWorkspace::default();
    ws.artifacts
        .insert(work.id().as_str().to_owned(), b"succeeded".to_vec());
    let runner = FakeRunner::new(OutcomeKind::Failed);
    let err = start_work(&mut store, &ws, &runner, &clock, work.id()).unwrap_err();
    assert!(matches!(err, AppError::OutcomeSchema(_)));
    assert_eq!(runner.runs.get(), 0);
}

#[test]
fn terminal_failed_outcome_cannot_be_reclassified() {
    let mut store = FakeStore::default();
    let ws = FakeWorkspace::default();
    let runner = FakeRunner::new(OutcomeKind::Failed);
    let clock = FakeClock { unix_ms: 1 };
    let work = ready_work(&mut store, &clock);
    start_work(&mut store, &ws, &runner, &clock, work.id()).unwrap();
    let replacement = Outcome::new(OUTCOME_SCHEMA_VERSION, OutcomeKind::TimedOut, "stub").unwrap();
    let err = complete(&mut store, &ws, &clock, work.id(), &replacement).unwrap_err();
    assert!(matches!(err, AppError::Conflict(_)));
    assert_eq!(ws.memory.borrow()[work.id().as_str()].len(), 1);
}

#[test]
fn outcome_profile_must_match_work_profile() {
    let mut store = FakeStore::default();
    let ws = FakeWorkspace::default();
    let clock = FakeClock { unix_ms: 1 };
    let work = ready_work(&mut store, &clock);
    let wrong = Outcome::new(OUTCOME_SCHEMA_VERSION, OutcomeKind::Succeeded, "other").unwrap();
    let err = complete(&mut store, &ws, &clock, work.id(), &wrong).unwrap_err();
    assert!(matches!(err, AppError::OutcomeSchema(_)));
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
fn parked_work_rejects_a_legacy_shared_outcome_artifact() {
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
    let err = start_work(&mut store, &ws, &runner, &clock, work.id()).unwrap_err();
    assert!(matches!(err, AppError::OutcomeSchema(_)));
    assert_eq!(runner.runs.get(), 0);
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
    fn run(&self, _request: &mut RunRequest<'_>) -> Result<WorkerExit, AppError> {
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

struct ChannelRunner {
    runs: Cell<u32>,
    reactions: Vec<ChannelReaction>,
    then: Option<OutcomeKind>,
}

impl WorkerRunner for ChannelRunner {
    fn run(&self, request: &mut RunRequest<'_>) -> Result<WorkerExit, AppError> {
        let i = self.runs.get() as usize;
        self.runs.set(self.runs.get() + 1);
        if i < self.reactions.len() {
            return match self.reactions[i] {
                ChannelReaction::Park => Ok(WorkerExit::Parked {
                    control_request_id: None,
                }),
                reaction => Err(AppError::Channel(reaction)),
            };
        }
        let kind = self.then.unwrap_or(OutcomeKind::Succeeded);
        Outcome::new(
            OUTCOME_SCHEMA_VERSION,
            kind,
            request.work.attributes().worker_profile(),
        )
        .map(WorkerExit::Completed)
        .map_err(AppError::from)
    }

    fn decode(&self, _bytes: &[u8]) -> Result<Outcome, AppError> {
        Err(AppError::worker("unused"))
    }
}

#[test]
fn channel_fail_completes_failed_without_retry() {
    let mut store = FakeStore::default();
    let ws = FakeWorkspace::default();
    let clock = FakeClock { unix_ms: 1 };
    let work = ready_work(&mut store, &clock);
    let runner = ChannelRunner {
        runs: Cell::new(0),
        reactions: vec![ChannelReaction::Fail],
        then: None,
    };
    let done = start_work_with(&mut store, &ws, &runner, &clock, work.id(), 3).unwrap();
    assert_eq!(done.status(), WorkStatus::Failed);
    assert_eq!(runner.runs.get(), 1);
    let last = store.events(work.id()).unwrap().pop().unwrap();
    assert_eq!(last.outcome_kind(), Some(OutcomeKind::ChannelError));
}

#[test]
fn channel_park_parks_the_same_work_without_complete() {
    let mut store = FakeStore::default();
    let ws = FakeWorkspace::default();
    let clock = FakeClock { unix_ms: 1 };
    let work = ready_work(&mut store, &clock);
    let runner = ChannelRunner {
        runs: Cell::new(0),
        reactions: vec![ChannelReaction::Park],
        then: None,
    };
    let done = start_work(&mut store, &ws, &runner, &clock, work.id()).unwrap();
    assert_eq!(done.status(), WorkStatus::Parked);
    assert_eq!(done.id(), work.id());
    assert!(ws.memory.borrow().get(work.id().as_str()).is_none());
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
        ]
    );
}

#[test]
fn channel_retry_respawns_inside_start_then_succeeds() {
    let mut store = FakeStore::default();
    let ws = FakeWorkspace::default();
    let clock = FakeClock { unix_ms: 1 };
    let work = ready_work(&mut store, &clock);
    let runner = ChannelRunner {
        runs: Cell::new(0),
        reactions: vec![ChannelReaction::Retry, ChannelReaction::Retry],
        then: Some(OutcomeKind::Succeeded),
    };
    let done = start_work_with(&mut store, &ws, &runner, &clock, work.id(), 2).unwrap();
    assert_eq!(done.status(), WorkStatus::Succeeded);
    assert_eq!(runner.runs.get(), 3);
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
            workengine_domain::EventKind::Completed,
        ]
    );
}

#[test]
fn channel_retry_exhaustion_fails_closed() {
    let mut store = FakeStore::default();
    let ws = FakeWorkspace::default();
    let clock = FakeClock { unix_ms: 1 };
    let work = ready_work(&mut store, &clock);
    let runner = ChannelRunner {
        runs: Cell::new(0),
        reactions: vec![ChannelReaction::Retry],
        then: None,
    };
    let done = start_work_with(&mut store, &ws, &runner, &clock, work.id(), 0).unwrap();
    assert_eq!(done.status(), WorkStatus::Failed);
    assert_eq!(runner.runs.get(), 1);
    let last = store.events(work.id()).unwrap().pop().unwrap();
    assert_eq!(last.outcome_kind(), Some(OutcomeKind::ChannelError));
}

#[test]
fn start_writes_the_goal_into_the_workspace_bind() {
    let mut store = FakeStore::default();
    let ws = FakeWorkspace::default();
    let runner = FakeRunner::new(OutcomeKind::Succeeded);
    let clock = FakeClock { unix_ms: 1 };
    let work = ready_work(&mut store, &clock);
    start_work(&mut store, &ws, &runner, &clock, work.id()).unwrap();
    assert_eq!(
        ws.goals
            .borrow()
            .get(work.id().as_str())
            .map(String::as_str),
        Some("do the thing")
    );
}

struct FakeInbound {
    records: Vec<InboundRecord>,
}

impl InboundSource for FakeInbound {
    fn source_id(&self) -> &str {
        "source-a"
    }

    fn poll(&mut self) -> Result<Vec<InboundRecord>, AppError> {
        Ok(self.records.clone())
    }
}

#[test]
fn inbound_requires_explicit_ready_and_is_idempotent() {
    let project = ProjectId::parse("project-a").unwrap();
    let pending = InboundRecord {
        record_id: "pending".to_owned(),
        signal: InboundSignal::Pending,
        project_id: project.clone(),
        repository: Some("repo-a".to_owned()),
        goal: "untrusted pending text".to_owned(),
        worker_profile: "stub".to_owned(),
        notification_target: Some("operator-a".to_owned()),
    };
    let ready = InboundRecord {
        record_id: "ready".to_owned(),
        signal: InboundSignal::Ready,
        ..pending.clone()
    };
    let mut source = FakeInbound {
        records: vec![pending, ready],
    };
    let mut store = FakeStore::default();
    let clock = FakeClock { unix_ms: 10 };
    let first = poll_inbound(&mut store, &mut source, &clock).unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].status(), WorkStatus::Ready);
    assert_eq!(first[0].attributes().project_id(), &project);
    assert_eq!(
        poll_inbound(&mut store, &mut source, &clock).unwrap().len(),
        0
    );
    assert_eq!(store.works.len(), 1);
}

#[derive(Default)]
struct FakePublicationStore {
    pending: Vec<Publication>,
    attempts: Vec<(i64, bool, Option<String>)>,
}

impl PublicationStore for FakePublicationStore {
    fn pending_publications(&self, limit: usize) -> Result<Vec<Publication>, AppError> {
        Ok(self.pending.iter().take(limit).cloned().collect())
    }

    fn record_publication_attempt(
        &mut self,
        publication_id: i64,
        delivered: bool,
        _attempted_at_unix_ms: u64,
        error: Option<&str>,
    ) -> Result<(), AppError> {
        self.attempts
            .push((publication_id, delivered, error.map(str::to_owned)));
        Ok(())
    }
}

struct FailingPublisher;

impl Publisher for FailingPublisher {
    fn publish(&mut self, _publication: &Publication) -> Result<(), AppError> {
        Err(AppError::worker("channel unavailable"))
    }
}

#[test]
fn publisher_failure_is_recorded_best_effort_without_a_status_writer() {
    let mut store = FakePublicationStore {
        pending: vec![Publication {
            publication_id: 1,
            kind: PublicationKind::ExternalInputRequired,
            work_id: WorkId::parse("work-a").unwrap(),
            project_id: ProjectId::parse("project-a").unwrap(),
            target: "operator-a".to_owned(),
            status: WorkStatus::Parked,
            created_at_unix_ms: 1,
            attempt_count: 0,
        }],
        attempts: Vec::new(),
    };
    let delivered = dispatch_publications(
        &mut store,
        &mut FailingPublisher,
        &FakeClock { unix_ms: 2 },
        10,
    )
    .unwrap();
    assert_eq!(delivered, 0);
    assert_eq!(store.attempts.len(), 1);
    assert!(!store.attempts[0].1);
}

#[derive(Default)]
struct FakeRemote {
    calls: usize,
}

impl RemoteMutation for FakeRemote {
    fn compare_and_swap(&mut self, request: &MutationRequest) -> Result<MutationResult, AppError> {
        self.calls += 1;
        if request.expected.revision != "rev-1" {
            return Err(AppError::Conflict("remote revision changed".to_owned()));
        }
        Ok(MutationResult {
            revision: "rev-2".to_owned(),
        })
    }
}

#[test]
fn remote_mutation_requires_matching_expected_context_and_cas() {
    let work = Work::new(
        WorkId::parse("work-a").unwrap(),
        workengine_domain::WorkAttributes::scoped(
            "goal",
            "stub",
            ProjectId::parse("project-a").unwrap(),
            Some("repo-a".to_owned()),
            None,
        )
        .unwrap(),
        1,
    )
    .unwrap();
    let mut remote = FakeRemote::default();
    let mismatch = MutationRequest {
        expected: ExpectedContext {
            project_id: ProjectId::parse("project-b").unwrap(),
            repository: "repo-a".to_owned(),
            revision: "rev-1".to_owned(),
        },
        change_digest: "sha256:change".to_owned(),
    };
    assert!(matches!(
        mutate_remote(&work, &mut remote, &mismatch),
        Err(AppError::Conflict(_))
    ));
    assert_eq!(remote.calls, 0);

    let matching = MutationRequest {
        expected: ExpectedContext {
            project_id: ProjectId::parse("project-a").unwrap(),
            repository: "repo-a".to_owned(),
            revision: "rev-1".to_owned(),
        },
        change_digest: "sha256:change".to_owned(),
    };
    assert_eq!(
        mutate_remote(&work, &mut remote, &matching)
            .unwrap()
            .revision,
        "rev-2"
    );
    assert_eq!(remote.calls, 1);
}
