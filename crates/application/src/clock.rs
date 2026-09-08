use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Time and deadlines. Worker hang detection sits outside the child.
pub trait Clock {
    fn unix_ms(&self) -> u64;
    fn now(&self) -> Instant;
}

/// Wall clock. `std::time` is not I/O.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn unix_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    fn now(&self) -> Instant {
        Instant::now()
    }
}
