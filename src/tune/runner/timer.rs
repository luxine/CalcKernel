use std::time::Instant;

#[derive(Clone, Copy)]
pub(super) struct MonotonicTimer(Instant);

impl MonotonicTimer {
    pub(super) fn start() -> Self {
        Self(Instant::now())
    }

    pub(super) fn elapsed_ns(&self) -> Result<u64, ()> {
        u64::try_from(self.0.elapsed().as_nanos()).map_err(|_| ())
    }

    pub(super) fn remaining(&self, duration: std::time::Duration) -> std::time::Duration {
        duration.saturating_sub(self.0.elapsed())
    }
}
