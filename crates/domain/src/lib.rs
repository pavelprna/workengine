//! Domain crate: identifiers, finite-state machine, closed outcomes.
//!
//! No I/O. No product or tracker names.

mod attempt_id;
mod error;
mod event;
mod execution;
mod execution_id;
mod outcome;
mod status;
mod work;
mod work_id;

pub use attempt_id::AttemptId;
pub use error::DomainError;
pub use event::{EVENT_SCHEMA_VERSION, EventKind, WorkEvent, replay};
pub use execution::{
    CONFIRMED_OUTCOME_SCHEMA_VERSION, ChannelPolicy, ConfirmedOutcome, ContentDigest,
    EXECUTION_SPEC_SCHEMA_VERSION, ExecutionSpec, RuntimeKind, SecretRef, SecretSource,
};
pub use execution_id::ExecutionId;
pub use outcome::{OUTCOME_SCHEMA_VERSION, Outcome, OutcomeKind};
pub use status::WorkStatus;
pub use work::{Apply, Work, WorkAttributes};
pub use work_id::WorkId;
