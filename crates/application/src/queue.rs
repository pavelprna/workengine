use workengine_domain::{ProjectId, WorkRelation};

use crate::clock::Clock;
use crate::error::AppError;
use crate::ports::{
    CaptureLease, CaptureRequest, QueueStore, QuotaLease, QuotaStore, RelationStore,
};

pub fn capture(
    store: &mut impl QueueStore,
    clock: &impl Clock,
    project_id: Option<&ProjectId>,
    worker_id: &str,
) -> Result<Option<CaptureLease>, AppError> {
    if worker_id.trim().is_empty() {
        return Err(AppError::Conflict("capture worker id is empty".to_owned()));
    }
    store.capture(
        &CaptureRequest {
            project_id,
            worker_id,
        },
        clock.unix_ms(),
    )
}

pub fn release_capture(store: &mut impl QueueStore, lease: &CaptureLease) -> Result<(), AppError> {
    store.release_capture(lease)
}

pub fn recover_captures(store: &mut impl QueueStore) -> Result<usize, AppError> {
    store.reclaim_captures()
}

pub fn add_relation(
    store: &mut impl RelationStore,
    relation: &WorkRelation,
) -> Result<(), AppError> {
    store.add_relation(relation)
}

pub fn configure_quota(
    store: &mut impl QuotaStore,
    resource: &str,
    limit: u32,
    expected_generation: Option<u64>,
) -> Result<u64, AppError> {
    validate_key(resource, "quota resource")?;
    store.configure_quota(resource, limit, expected_generation)
}

pub fn acquire_quota(
    store: &mut impl QuotaStore,
    resource: &str,
    holder: &str,
    units: u32,
) -> Result<QuotaLease, AppError> {
    validate_key(resource, "quota resource")?;
    validate_key(holder, "quota holder")?;
    if units == 0 {
        return Err(AppError::Conflict(
            "quota units must be positive".to_owned(),
        ));
    }
    store.acquire_quota(resource, holder, units)
}

pub fn release_quota(store: &mut impl QuotaStore, lease: &QuotaLease) -> Result<(), AppError> {
    store.release_quota(lease)
}

fn validate_key(value: &str, label: &str) -> Result<(), AppError> {
    if value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(AppError::Conflict(format!("{label} is invalid")));
    }
    Ok(())
}
