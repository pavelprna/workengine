use workengine_domain::{WorkId, WorkStatus};

use crate::error::AppError;
use crate::ports::WorkStore;

/// Oldest `ready`, else oldest `parked`. Does not spawn.
pub fn next(store: &impl WorkStore) -> Result<Option<WorkId>, AppError> {
    let mut works = store.list()?;
    works.sort_by_key(|w| w.created_at_unix_ms());
    if let Some(work) = works.iter().find(|w| w.status() == WorkStatus::Ready) {
        return Ok(Some(work.id().clone()));
    }
    Ok(works
        .iter()
        .find(|w| w.status() == WorkStatus::Parked)
        .map(|w| w.id().clone()))
}
