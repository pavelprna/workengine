/// Wall-clock timestamps for Work and events.
/// Hang and budget deadlines belong to the WorkerRunner adapter, not this trait.
pub trait Clock {
    fn unix_ms(&self) -> u64;
}

/// Wall clock. `std::time` is not I/O.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn unix_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}
