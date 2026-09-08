use workengine_domain::{Apply, Work, WorkEvent, WorkId, WorkStatus};

use crate::error::AppError;
use crate::ports::WorkStore;

/// Pause leftover `running` Work. Does not kill a process group.
pub fn park(store: &mut impl WorkStore, id: &WorkId) -> Result<Work, AppError> {
    let mut work = store
        .get(id)?
        .ok_or_else(|| AppError::NotFound(id.clone()))?;
    let from = work.status();
    match work.park()? {
        Apply::Idempotent { .. } => Ok(work),
        Apply::Changed { .. } => {
            store.put(&work, WorkEvent::parked(&work, from))?;
            Ok(work)
        }
    }
}

/// First-slice resume: leftover `running` (no committed complete) returns to the queue as `parked`.
pub fn recover_unconfirmed(store: &mut impl WorkStore) -> Result<Vec<WorkId>, AppError> {
    let leftover: Vec<WorkId> = store
        .list()?
        .into_iter()
        .filter(|w| w.status() == WorkStatus::Running)
        .map(|w| w.id().clone())
        .collect();
    let mut parked = Vec::new();
    for id in leftover {
        let work = park(store, &id)?;
        parked.push(work.id().clone());
    }
    Ok(parked)
}
