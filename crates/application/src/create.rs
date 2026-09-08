use workengine_domain::{Work, WorkAttributes, WorkEvent, WorkId};

use crate::clock::Clock;
use crate::error::AppError;
use crate::ports::WorkStore;

/// Create Work as `ready`. Does not spawn a Worker.
pub fn create(
    store: &mut impl WorkStore,
    clock: &impl Clock,
    goal: impl Into<String>,
    worker_profile: impl Into<String>,
) -> Result<Work, AppError> {
    let id = WorkId::parse(uuid::Uuid::new_v4().to_string())?;
    let attributes = WorkAttributes::new(goal, worker_profile)?;
    let work = Work::new(id, attributes, clock.unix_ms())?;
    if store.get(work.id())?.is_some() {
        return Err(AppError::Conflict(work.id().to_string()));
    }
    store.put(&work, WorkEvent::created(&work))?;
    Ok(work)
}
