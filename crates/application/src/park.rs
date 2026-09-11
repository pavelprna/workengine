use workengine_domain::{Apply, Work, WorkEvent, WorkId, WorkStatus};

use crate::clock::Clock;
use crate::error::AppError;
use crate::ports::{AttemptState, WorkStore};

/// Historical recovery transition. Live operator park uses `request_control`.
pub fn park(store: &mut impl WorkStore, clock: &impl Clock, id: &WorkId) -> Result<Work, AppError> {
    let mut work = store
        .get(id)?
        .ok_or_else(|| AppError::NotFound(id.clone()))?;
    let from = work.status();
    match work.park()? {
        Apply::Idempotent { .. } => Ok(work),
        Apply::Changed { .. } => {
            store.put(&work, WorkEvent::parked(&work, from, clock.unix_ms()))?;
            Ok(work)
        }
    }
}

/// Reclaim every unconfirmed lease before the daemon accepts new controls.
pub fn recover_unconfirmed(
    store: &mut impl WorkStore,
    clock: &impl Clock,
) -> Result<Vec<WorkId>, AppError> {
    let leftover: Vec<WorkId> = store
        .list()?
        .into_iter()
        .filter(|w| w.status() == WorkStatus::Running)
        .map(|w| w.id().clone())
        .collect();
    let mut parked = Vec::new();
    for id in leftover {
        let mut work = store
            .get(&id)?
            .ok_or_else(|| AppError::NotFound(id.clone()))?;
        let active = store
            .executions(&id)?
            .into_iter()
            .rev()
            .find_map(|execution| {
                execution
                    .attempts
                    .last()
                    .filter(|attempt| attempt.state == AttemptState::Active)
                    .map(|attempt| (execution.execution_id, attempt.attempt_id.clone()))
            });
        let Some((execution_id, attempt_id)) = active else {
            let work = park(store, clock, &id)?;
            parked.push(work.id().clone());
            continue;
        };
        let from = work.status();
        work.park()?;
        store.reclaim_attempt(
            &work,
            WorkEvent::parked(&work, from, clock.unix_ms()),
            &execution_id,
            &attempt_id,
        )?;
        parked.push(work.id().clone());
    }
    Ok(parked)
}
