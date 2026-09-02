use std::time::Instant;

pub(super) struct MonotonicTimer(Instant);

impl MonotonicTimer {
    pub(super) fn start() -> Self {
        Self(Instant::now())
    }

    pub(super) fn elapsed_ns(&self) -> Result<u64, ()> {
        u64::try_from(self.0.elapsed().as_nanos()).map_err(|_| ())
    }

    pub(super) fn reached(&self, duration: std::time::Duration) -> bool {
        self.0.elapsed() >= duration
    }
}
