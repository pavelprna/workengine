use workengine_domain::{Work, WorkAttributes, WorkEvent, WorkId};

use crate::clock::Clock;
use crate::error::AppError;
use crate::ports::{
    InboundSignal, InboundSource, InboundStore, MutationRequest, MutationResult, PublicationStore,
    Publisher, RemoteMutation,
};

/// Poll exactly one source. Scheduling sources independently belongs to an adapter.
pub fn poll_inbound(
    store: &mut impl InboundStore,
    source: &mut impl InboundSource,
    clock: &impl Clock,
) -> Result<Vec<Work>, AppError> {
    let source_id = source.source_id().to_owned();
    if source_id.trim().is_empty() {
        return Err(AppError::Conflict("inbound source id is empty".to_owned()));
    }
    let mut created = Vec::new();
    for record in source.poll()? {
        if record.record_id.trim().is_empty()
            || record.record_id.len() > 256
            || record.record_id.chars().any(char::is_control)
        {
            return Err(AppError::Conflict(
                "inbound record id is invalid".to_owned(),
            ));
        }
        if record.signal != InboundSignal::Ready {
            continue;
        }
        let work = Work::new(
            WorkId::parse(uuid::Uuid::new_v4().to_string())?,
            WorkAttributes::scoped(
                record.goal,
                record.worker_profile,
                record.project_id,
                record.repository,
                record.notification_target,
            )?,
            clock.unix_ms(),
        )?;
        if store.put_inbound(
            &source_id,
            &record.record_id,
            &work,
            WorkEvent::created(&work),
        )? {
            created.push(work);
        }
    }
    Ok(created)
}

/// Fetches before calling external code and records an attempt afterwards.
/// No store transaction is held while the publisher may block or fail.
pub fn dispatch_publications(
    store: &mut impl PublicationStore,
    publisher: &mut impl Publisher,
    clock: &impl Clock,
    limit: usize,
) -> Result<usize, AppError> {
    let pending = store.pending_publications(limit)?;
    let mut delivered = 0;
    for publication in pending {
        let result = publisher.publish(&publication);
        let error = result.as_ref().err().map(ToString::to_string);
        let succeeded = result.is_ok();
        store.record_publication_attempt(
            publication.publication_id,
            succeeded,
            clock.unix_ms(),
            error.as_deref(),
        )?;
        delivered += usize::from(succeeded);
    }
    Ok(delivered)
}

pub fn mutate_remote(
    work: &Work,
    remote: &mut impl RemoteMutation,
    request: &MutationRequest,
) -> Result<MutationResult, AppError> {
    if work.attributes().project_id() != &request.expected.project_id
        || work.attributes().repository() != Some(request.expected.repository.as_str())
        || request.expected.revision.trim().is_empty()
        || request.change_digest.trim().is_empty()
    {
        return Err(AppError::Conflict(
            "remote mutation expected context does not match Work".to_owned(),
        ));
    }
    remote.compare_and_swap(request)
}
