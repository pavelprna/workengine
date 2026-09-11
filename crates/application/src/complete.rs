use workengine_domain::{Apply, Outcome, Work, WorkEvent, WorkId};

use crate::clock::Clock;
use crate::error::AppError;
use crate::ports::{WorkStore, WorkspaceFactory};

/// Apply a closed outcome. Repeat does not append a second event or memory line.
/// A retry after a failed memory write still records memory.
pub fn complete(
    store: &mut impl WorkStore,
    workspaces: &impl WorkspaceFactory,
    clock: &impl Clock,
    id: &WorkId,
    outcome: &Outcome,
) -> Result<Work, AppError> {
    let mut work = store
        .get(id)?
        .ok_or_else(|| AppError::NotFound(id.clone()))?;
    if outcome.worker_profile() != work.attributes().worker_profile() {
        return Err(AppError::outcome_schema(format!(
            "outcome profile {} does not match Work profile {}",
            outcome.worker_profile(),
            work.attributes().worker_profile()
        )));
    }
    if matches!(
        work.status(),
        workengine_domain::WorkStatus::Succeeded | workengine_domain::WorkStatus::Failed
    ) && store
        .events(id)?
        .last()
        .and_then(|event| event.outcome_kind())
        .is_some_and(|kind| kind != outcome.kind())
    {
        return Err(AppError::Conflict(format!(
            "terminal outcome for {id} differs from the recorded outcome"
        )));
    }
    let from = work.status();
    match work.complete(outcome)? {
        Apply::Idempotent { status } => {
            workspaces.record_memory(id, work.attributes().project_id(), status, outcome.kind())?;
            Ok(work)
        }
        Apply::Changed { to, .. } => {
            store.put(
                &work,
                WorkEvent::completed(&work, from, outcome.kind(), clock.unix_ms()),
            )?;
            workspaces.record_memory(id, work.attributes().project_id(), to, outcome.kind())?;
            Ok(work)
        }
    }
}
