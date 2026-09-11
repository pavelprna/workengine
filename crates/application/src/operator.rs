use workengine_domain::{WorkId, WorkStatus};

use crate::clock::Clock;
use crate::error::AppError;
use crate::ports::{
    ControlDirective, ControlKind, OperatorInput, OperatorInputKind, WorkStore, WorkspaceFactory,
};

pub fn request_control(
    store: &mut impl WorkStore,
    clock: &impl Clock,
    id: &WorkId,
    kind: ControlKind,
) -> Result<ControlDirective, AppError> {
    let work = store
        .get(id)?
        .ok_or_else(|| AppError::NotFound(id.clone()))?;
    if work.status() != WorkStatus::Running {
        return Err(AppError::Conflict(format!("Work {id} is not running")));
    }
    store.request_control(id, kind, clock.unix_ms())
}

pub fn record_operator_input(
    store: &mut impl WorkStore,
    workspaces: &impl WorkspaceFactory,
    clock: &impl Clock,
    id: &WorkId,
    kind: OperatorInputKind,
    body: String,
) -> Result<OperatorInput, AppError> {
    if body.trim().is_empty() {
        return Err(AppError::Conflict("operator input is empty".to_owned()));
    }
    let work = store
        .get(id)?
        .ok_or_else(|| AppError::NotFound(id.clone()))?;
    if work.status() != WorkStatus::Parked {
        return Err(AppError::Conflict(format!("Work {id} is not parked")));
    }
    let input = store.append_operator_input(id, kind, &body, clock.unix_ms())?;
    workspaces.record_operator_input(id, &input)?;
    Ok(input)
}
