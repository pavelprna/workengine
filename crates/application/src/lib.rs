//! Application crate: use cases and port traits.
//!
//! Depends only on `workengine-domain`. Adapters own I/O.

mod clock;
mod complete;
mod create;
mod error;
mod next;
mod park;
mod ports;
mod start;

pub use clock::{Clock, SystemClock};
pub use complete::complete;
pub use create::create;
pub use error::{AppError, ChannelReaction};
pub use next::next;
pub use park::{park, recover_unconfirmed};
pub use ports::{
    BindRequest, RunRequest, SequencedEvent, StartRequest, WorkQuery, WorkStore, WorkerRunner,
    WorkspaceFactory,
};
pub use start::start;

#[cfg(test)]
mod tests;
