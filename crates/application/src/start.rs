use workengine_domain::{
    OUTCOME_SCHEMA_VERSION, Outcome, OutcomeKind, Work, WorkEvent, WorkStatus,
};

use crate::clock::Clock;
use crate::complete::complete;
use crate::error::{AppError, ChannelReaction};
use crate::park::park;
use crate::ports::{
    BindRequest, RunRequest, StartRequest, WorkStore, WorkerRunner, WorkspaceFactory,
};

/// Bind a workspace, spawn a Worker, then complete from its returned outcome.
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
    let artifact = workspaces.read_artifact(id)?;
    if artifact.is_some() {
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
    if work.status() == WorkStatus::Running {
        park(store, clock, id)?;
        work = store
            .get(id)?
            .ok_or_else(|| AppError::NotFound(id.clone()))?;
        work.bind_workspace(root.to_string_lossy().into_owned())?;
    }

    if work.status() != WorkStatus::Running {
        let from = work.status();
        work.start()?;
        store.put(&work, WorkEvent::started(&work, from, clock.unix_ms()))?;
    }

    let profile = work.attributes().worker_profile().to_owned();
    let mut remaining = request.retry_limit;
    loop {
        match runner.run(&RunRequest {
            work: &work,
            workspace_root: &root,
            budget: request.budget,
        }) {
            Ok(outcome) => return complete(store, workspaces, clock, id, &outcome),
            Err(err @ AppError::OutcomeSchema(_)) => return Err(err),
            Err(AppError::Channel(ChannelReaction::Park)) => {
                return park(store, clock, id);
            }
            Err(AppError::Channel(ChannelReaction::Retry)) if remaining > 0 => {
                remaining -= 1;
            }
            Err(AppError::Channel(ChannelReaction::Retry | ChannelReaction::Fail)) | Err(_) => {
                return persist_channel(store, workspaces, clock, id, &profile);
            }
        }
    }
}

fn persist_channel(
    store: &mut impl WorkStore,
    workspaces: &impl WorkspaceFactory,
    clock: &impl Clock,
    id: &workengine_domain::WorkId,
    profile: &str,
) -> Result<Work, AppError> {
    let outcome = Outcome::new(OUTCOME_SCHEMA_VERSION, OutcomeKind::ChannelError, profile)?;
    complete(store, workspaces, clock, id, &outcome)
}
