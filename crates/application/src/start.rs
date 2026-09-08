use std::time::Duration;

use workengine_domain::{
    OUTCOME_SCHEMA_VERSION, Outcome, OutcomeKind, Work, WorkEvent, WorkId, WorkStatus,
};

use crate::complete::complete;
use crate::error::AppError;
use crate::park::park;
use crate::ports::{RunRequest, WorkStore, WorkerRunner, WorkspaceFactory};

/// Bind a workspace, spawn unless an outcome artifact already exists, then complete.
pub fn start(
    store: &mut impl WorkStore,
    workspaces: &impl WorkspaceFactory,
    runner: &impl WorkerRunner,
    id: &WorkId,
    budget: Duration,
) -> Result<Work, AppError> {
    let mut work = store
        .get(id)?
        .ok_or_else(|| AppError::NotFound(id.clone()))?;
    let root = workspaces.bind(id)?;
    work.bind_workspace(root.to_string_lossy().into_owned());

    let artifact = workspaces.read_artifact(id)?;
    if work.status() == WorkStatus::Running && artifact.is_none() {
        park(store, id)?;
        work = store
            .get(id)?
            .ok_or_else(|| AppError::NotFound(id.clone()))?;
        work.bind_workspace(root.to_string_lossy().into_owned());
    }

    if let Some(bytes) = artifact {
        let outcome = runner.decode(&bytes)?;
        if work.status() == WorkStatus::Ready {
            let from = work.status();
            work.start()?;
            store.put(&work, WorkEvent::started(&work, from))?;
        }
        return complete(store, workspaces, id, &outcome);
    }

    if work.status() != WorkStatus::Running {
        let from = work.status();
        work.start()?;
        store.put(&work, WorkEvent::started(&work, from))?;
    }

    let profile = work.attributes().worker_profile().to_owned();
    let outcome = match runner.run(&RunRequest {
        work: &work,
        workspace_root: &root,
        budget,
    }) {
        Ok(outcome) => outcome,
        Err(err @ AppError::OutcomeSchema(_)) => return Err(err),
        Err(_) => {
            return persist_channel(store, workspaces, id, &profile);
        }
    };

    complete(store, workspaces, id, &outcome)
}

fn persist_channel(
    store: &mut impl WorkStore,
    workspaces: &impl WorkspaceFactory,
    id: &WorkId,
    profile: &str,
) -> Result<Work, AppError> {
    let outcome = Outcome::new(OUTCOME_SCHEMA_VERSION, OutcomeKind::ChannelError, profile)?;
    complete(store, workspaces, id, &outcome)
}
