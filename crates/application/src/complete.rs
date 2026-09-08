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
    let from = work.status();
    match work.complete(outcome)? {
        Apply::Idempotent { status } => {
            workspaces.record_memory(id, status, outcome.kind())?;
            Ok(work)
        }
        Apply::Changed { to, .. } => {
            store.put(
                &work,
                WorkEvent::completed(&work, from, outcome.kind(), clock.unix_ms()),
            )?;
            workspaces.record_memory(id, to, outcome.kind())?;
            Ok(work)
        }
    }
}
