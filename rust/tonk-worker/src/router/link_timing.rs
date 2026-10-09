//! Local, secret-free stage timings for an explicit connection attempt.

use std::sync::atomic::{AtomicU32, Ordering};
use web_time::Instant;

pub(super) struct LinkTiming {
    id: u32,
    operation: &'static str,
    stage: &'static str,
    started: Instant,
    since: Instant,
}

impl LinkTiming {
    pub(super) fn new(operation: &'static str, stage: &'static str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(1);
        let started = Instant::now();
        Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            operation,
            stage,
            started,
            since: started,
        }
    }

    pub(super) fn next(&mut self, stage: &'static str) {
        self.record("completed");
        self.stage = stage;
        self.since = Instant::now();
    }

    fn record(&self, outcome: &str) {
        tonk_common::log!(
            "link-timing id={} operation={} stage={} elapsed_ms={} total_ms={} outcome={}",
            self.id,
            self.operation,
            self.stage,
            self.since.elapsed().as_millis(),
            self.started.elapsed().as_millis(),
            outcome,
        );
    }
}

impl Drop for LinkTiming {
    fn drop(&mut self) {
        // Also records the last stage on an early error or cancellation.
        self.record("returned");
    }
}
