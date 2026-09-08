use workengine_domain::{Apply, Outcome, Work, WorkEvent, WorkId};

use crate::error::AppError;
use crate::ports::{WorkStore, WorkspaceFactory};

/// Apply a closed outcome. Repeat does not append a second event.
pub fn complete(
    store: &mut impl WorkStore,
    workspaces: &impl WorkspaceFactory,
    id: &WorkId,
    outcome: &Outcome,
) -> Result<Work, AppError> {
    let mut work = store
        .get(id)?
        .ok_or_else(|| AppError::NotFound(id.clone()))?;
    let from = work.status();
    match work.complete(outcome)? {
        Apply::Idempotent { .. } => Ok(work),
        Apply::Changed { to, .. } => {
            store.put(&work, WorkEvent::completed(&work, from, outcome.kind()))?;
            let entry = format!(
                r#"{{"schemaVersion":1,"workId":"{}","status":"{}","outcomeKind":"{}"}}"#,
                work.id(),
                to.as_str(),
                outcome.kind().as_str()
            );
            workspaces.record_memory(id, &entry)?;
            Ok(work)
        }
    }
}
