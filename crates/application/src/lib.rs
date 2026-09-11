//! Application crate: use cases and port traits.
//!
//! Depends only on `workengine-domain`. Adapters own I/O.

mod clock;
mod complete;
mod create;
mod error;
mod integrations;
mod next;
mod operator;
mod park;
mod ports;
mod queue;
mod start;

pub use clock::{Clock, SystemClock};
pub use complete::complete;
pub use create::{create, create_scoped};
pub use error::{AppError, ChannelReaction};
pub use integrations::{dispatch_publications, mutate_remote, poll_inbound};
pub use next::next;
pub use operator::{record_operator_input, request_control};
pub use park::{park, recover_unconfirmed};
pub use ports::{
    AttemptClaim, AttemptObservation, AttemptRecorder, AttemptState, BindRequest, CaptureLease,
    CaptureRequest, ConfirmedOutcomeObservation, ControlDirective, ControlKind,
    DiscardAttemptRecorder, ExecutionObservation, ExecutionSpecObservation, ExpectedContext,
    InboundRecord, InboundSignal, InboundSource, InboundStore, MutationRequest, MutationResult,
    OperatorInput, OperatorInputKind, ProcessEvent, ProcessRecordObservation, Publication,
    PublicationKind, PublicationStore, Publisher, QueueStore, QuotaLease, QuotaStore,
    RelationStore, RemoteMutation, RunRequest, SecretRefObservation, SequencedEvent, StartRequest,
    WorkQuery, WorkStore, WorkerExit, WorkerRunner, WorkspaceFactory,
};
pub use queue::{
    acquire_quota, add_relation, capture, configure_quota, recover_captures, release_capture,
    release_quota,
};
pub use start::start;

#[cfg(test)]
mod tests;
