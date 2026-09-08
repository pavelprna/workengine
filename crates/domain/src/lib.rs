//! Domain crate: identifiers, finite-state machine, closed outcomes.
//!
//! No I/O. No product or tracker names.

mod error;
mod event;
mod outcome;
mod status;
mod work;
mod work_id;

pub use error::DomainError;
pub use event::{EVENT_SCHEMA_VERSION, EventKind, WorkEvent, replay};
pub use outcome::{OUTCOME_SCHEMA_VERSION, Outcome, OutcomeKind};
pub use status::WorkStatus;
pub use work::{Apply, Work, WorkAttributes};
pub use work_id::WorkId;
