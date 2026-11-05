//! Local, secret-free stage timings for an explicit connection attempt.

use std::sync::atomic::{AtomicU32, Ordering};
use web_time::Instant;

pub(super) struct LinkTiming {
    id: u32,
    operation: &'static str,
    stage: &'static str,
    started: Instant,
    since: Instant,
    requests: Requests,
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
            requests: Requests::now(),
        }
    }

    pub(super) fn next(&mut self, stage: &'static str) {
        self.record("completed");
        self.stage = stage;
        self.since = Instant::now();
        self.requests = Requests::now();
    }

    fn record(&self, outcome: &str) {
        tonk_common::log!(
            "link-timing id={} operation={} stage={} elapsed_ms={} total_ms={} outcome={}{}",
            self.id,
            self.operation,
            self.stage,
            self.since.elapsed().as_millis(),
            self.started.elapsed().as_millis(),
            outcome,
            self.requests.since(),
        );
    }
}

impl Drop for LinkTiming {
    fn drop(&mut self) {
        // Also records the last stage on an early error or cancellation.
        self.record("returned");
    }
}

/// How many remote requests this worker had made when a stage began, read
/// from the worker's resource timing entries.
///
/// A stage that reads remote content pays a round trip per tree node it
/// lacks, each an access-service invocation for a presigned URL and the
/// storage read it signs, so counting both says whether a slow stage was
/// many requests or slow ones. Requests other work makes at the same time
/// (a sync of a space, say) are counted too.
struct Requests(Option<usize>);

impl Requests {
    fn now() -> Self {
        Self(resource_entries().map(|entries| entries.length() as usize))
    }

    /// The requests made since, by kind, as a suffix for a timing line.
    fn since(&self) -> String {
        let (Some(from), Some(entries)) = (self.0, resource_entries()) else {
            return String::new();
        };
        let origin = own_origin();
        let (mut access, mut storage) = (0, 0);
        for index in from..entries.length() as usize {
            let name = js_sys::Reflect::get(&entries.get(index as u32), &"name".into())
                .ok()
                .and_then(|name| name.as_string())
                .unwrap_or_default();
            if name.contains("/ucan/") {
                access += 1;
            } else if origin.is_empty() || !name.starts_with(&origin) {
                storage += 1;
            }
        }
        format!(" access_requests={access} storage_requests={storage}")
    }
}

/// The worker's resource timing entries, once the buffer is large enough
/// to hold a long stage's requests. Nothing off the worker.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn resource_entries() -> Option<js_sys::Array> {
    use wasm_bindgen::JsCast as _;

    // The default buffer holds 250 entries and then stops recording, which
    // one cold read of an account branch outgrows.
    const BUFFER: u32 = 100_000;
    static SIZED: std::sync::Once = std::sync::Once::new();

    let performance = js_sys::Reflect::get(&js_sys::global(), &"performance".into()).ok()?;
    let call = |method: &str| {
        js_sys::Reflect::get(&performance, &method.into())
            .ok()?
            .dyn_into::<js_sys::Function>()
            .ok()
    };
    SIZED.call_once(|| {
        if let Some(resize) = call("setResourceTimingBufferSize") {
            let _ = resize.call1(&performance, &BUFFER.into());
        }
    });
    call("getEntriesByType")?
        .call1(&performance, &"resource".into())
        .ok()?
        .dyn_into::<js_sys::Array>()
        .ok()
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn resource_entries() -> Option<js_sys::Array> {
    None
}

/// The worker's own origin, whose requests are neither kind.
fn own_origin() -> String {
    js_sys::Reflect::get(&js_sys::global(), &"location".into())
        .and_then(|location| js_sys::Reflect::get(&location, &"origin".into()))
        .ok()
        .and_then(|origin| origin.as_string())
        .unwrap_or_default()
}
