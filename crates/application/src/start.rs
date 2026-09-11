use std::time::{Duration, Instant};

use workengine_domain::{
    AttemptId, CONFIRMED_OUTCOME_SCHEMA_VERSION, ConfirmedOutcome, ExecutionId,
    OUTCOME_SCHEMA_VERSION, Outcome, OutcomeKind, Work, WorkEvent,
};

use crate::clock::Clock;
use crate::error::{AppError, ChannelReaction};
use crate::ports::{
    AttemptClaim, BindRequest, RunRequest, StartRequest, WorkStore, WorkerRunner, WorkspaceFactory,
};

/// Claim an attempt, bind a workspace, spawn a Worker, and confirm its outcome.
pub fn start(
    store: &mut impl WorkStore,
    workspaces: &impl WorkspaceFactory,
    runner: &impl WorkerRunner,
    clock: &impl Clock,
    request: &StartRequest<'_>,
) -> Result<Work, AppError> {
    let id = request.id;
    let mut work = store
        .get(id)?
        .ok_or_else(|| AppError::NotFound(id.clone()))?;
    if request.execution_spec.work_id() != id
        || request.execution_spec.worker_profile() != work.attributes().worker_profile()
    {
        return Err(AppError::Conflict(
            "execution spec does not belong to this Work and profile".to_owned(),
        ));
    }
    if workspaces.read_artifact(id)?.is_some() {
        return Err(AppError::outcome_schema(
            "shared workspace outcome artifacts are unsupported; a protected attempt artifact is required",
        ));
    }

    let root = workspaces.bind(&BindRequest {
        work_id: id,
        goal: work.attributes().goal(),
        checkout: request.checkout,
    })?;
    work.bind_workspace(root.to_string_lossy().into_owned())?;
    let from = work.status();
    work.start()?;

    let execution_id = generated_execution_id()?;
    let mut attempt_id = generated_attempt_id()?;
    let started_at = clock.unix_ms();
    store.claim_attempt(&AttemptClaim {
        execution_id: &execution_id,
        attempt_id: &attempt_id,
        spec: request.execution_spec,
        work: &work,
        event: WorkEvent::started(&work, from, started_at),
        started_at_unix_ms: started_at,
    })?;

    let budget = Duration::from_millis(request.execution_spec.wall_clock_budget_ms());
    let started = Instant::now();
    let mut remaining_retries = request.execution_spec.retry_limit();
    loop {
        let remaining_budget = budget.saturating_sub(started.elapsed());
        if remaining_budget.is_zero() {
            let outcome = Outcome::new(
                OUTCOME_SCHEMA_VERSION,
                OutcomeKind::BudgetExceeded,
                work.attributes().worker_profile(),
            )?;
            return confirm(
                store,
                workspaces,
                clock,
                &mut work,
                request,
                &execution_id,
                &attempt_id,
                outcome,
            );
        }
        match runner.run(&RunRequest {
            work: &work,
            workspace_root: &root,
            budget: remaining_budget,
        }) {
            Ok(outcome) => {
                return confirm(
                    store,
                    workspaces,
                    clock,
                    &mut work,
                    request,
                    &execution_id,
                    &attempt_id,
                    outcome,
                );
            }
            Err(err @ AppError::OutcomeSchema(_)) => {
                park_claim(store, clock, &mut work, &execution_id, &attempt_id)?;
                return Err(err);
            }
            Err(AppError::Channel(ChannelReaction::Park)) => {
                return park_claim(store, clock, &mut work, &execution_id, &attempt_id);
            }
            Err(AppError::Channel(ChannelReaction::Retry)) if remaining_retries > 0 => {
                remaining_retries -= 1;
                let next_attempt_id = generated_attempt_id()?;
                store.retry_attempt(
                    &execution_id,
                    &attempt_id,
                    &next_attempt_id,
                    clock.unix_ms(),
                )?;
                attempt_id = next_attempt_id;
            }
            Err(AppError::Channel(ChannelReaction::Retry | ChannelReaction::Fail)) | Err(_) => {
                let outcome = Outcome::new(
                    OUTCOME_SCHEMA_VERSION,
                    OutcomeKind::ChannelError,
                    work.attributes().worker_profile(),
                )?;
                return confirm(
                    store,
                    workspaces,
                    clock,
                    &mut work,
                    request,
                    &execution_id,
                    &attempt_id,
                    outcome,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn confirm(
    store: &mut impl WorkStore,
    workspaces: &impl WorkspaceFactory,
    clock: &impl Clock,
    work: &mut Work,
    request: &StartRequest<'_>,
    execution_id: &ExecutionId,
    attempt_id: &AttemptId,
    outcome: Outcome,
) -> Result<Work, AppError> {
    let confirmed_at = clock.unix_ms();
    let confirmed = ConfirmedOutcome::new(
        CONFIRMED_OUTCOME_SCHEMA_VERSION,
        execution_id.clone(),
        attempt_id.clone(),
        request.execution_spec,
        outcome,
        confirmed_at,
    )?;
    let from = work.status();
    work.complete(confirmed.outcome())?;
    store.confirm_attempt(
        work,
        WorkEvent::completed(work, from, confirmed.outcome().kind(), confirmed_at),
        &confirmed,
    )?;
    workspaces.record_memory(work.id(), work.status(), confirmed.outcome().kind())?;
    Ok(work.clone())
}

fn park_claim(
    store: &mut impl WorkStore,
    clock: &impl Clock,
    work: &mut Work,
    execution_id: &ExecutionId,
    attempt_id: &AttemptId,
) -> Result<Work, AppError> {
    let from = work.status();
    work.park()?;
    store.park_attempt(
        work,
        WorkEvent::parked(work, from, clock.unix_ms()),
        execution_id,
        attempt_id,
    )?;
    Ok(work.clone())
}

fn generated_execution_id() -> Result<ExecutionId, AppError> {
    ExecutionId::parse(format!("execution-{}", uuid::Uuid::new_v4())).map_err(AppError::from)
}

fn generated_attempt_id() -> Result<AttemptId, AppError> {
    AttemptId::parse(format!("attempt-{}", uuid::Uuid::new_v4())).map_err(AppError::from)
}
