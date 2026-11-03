//! Counters and durations a running Tonk context keeps about itself.
//!
//! Every JavaScript realm (the page, each guest frame, the service worker)
//! and every native process has one registry. Code that does something worth
//! counting calls [`count`] or [`record`] with a name from [`names`]; a test,
//! a trace capture or a gate reads the registry back with [`snapshot`].
//!
//! The registry measures, it never decides: nothing reads it to change
//! behaviour. That keeps a counter safe to add anywhere, and it keeps the
//! numbers comparable between a scenario run in CI and a trace captured from
//! a real session.
//!
//! On the web, [`install_global`] exposes the registry to scripts in that
//! realm as `globalThis.__tonkDiagnostics`, with `snapshot()` returning the
//! object [`snapshot`] describes and `reset()` clearing it. The service
//! worker's registry is served by the worker's own diagnostics route, since a
//! page cannot reach the worker's globals.
//!
//! ```
//! use tonk_common::diagnostics::{self, names};
//!
//! diagnostics::reset();
//! diagnostics::count(names::SITE_GUEST_BOOT_MOUNT);
//! diagnostics::record(names::WORKER_LOCK_READ_WAIT, 12.0);
//!
//! let snapshot = diagnostics::snapshot();
//! assert_eq!(snapshot.counters[names::SITE_GUEST_BOOT_MOUNT], 1);
//! assert_eq!(snapshot.durations[names::WORKER_LOCK_READ_WAIT].count, 1);
//! ```

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use std::cell::RefCell;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The names every Tonk surface records under.
///
/// A name says where it was counted and what happened, dot-separated,
/// with the cause last where one event has several causes. Trace capture and
/// the perf gates key on these strings, so renaming one is a change to their
/// baselines: add a new name instead of reusing an old one for a new meaning.
pub mod names {
    /// `<tonk-site>` built a guest frame because the element was connected
    /// for the first time.
    pub const SITE_GUEST_BOOT_MOUNT: &str = "site.guest_boot.mount";
    /// `<tonk-site>` built a guest frame because the element was connected
    /// again after being disconnected: the same element, removed and
    /// re-inserted.
    pub const SITE_GUEST_BOOT_REMOUNT: &str = "site.guest_boot.remount";
    /// `<tonk-site>` rebuilt its guest frame because `with` or `allow`
    /// changed, or the frame it had was gone: a space switch.
    pub const SITE_GUEST_BOOT_NAVIGATION: &str = "site.guest_boot.navigation";
    /// `<tonk-site>` built a guest frame again because the person asked a
    /// site that failed to load to try again.
    pub const SITE_GUEST_BOOT_RETRY: &str = "site.guest_boot.retry";
    /// `<tonk-site>` showed that its page could not load: the guest did not
    /// answer the handshake in time, or its `tonk:load` claim failed twice.
    pub const SITE_LOAD_FAILED: &str = "site.load_failed";
    /// `<tonk-site>` re-routed a live guest frame in place, with no rebuild.
    pub const SITE_REROUTE: &str = "site.reroute";

    /// `<tonk-display>` removed content it had kept through a restart (a
    /// changed `entity`, `model` or `view` attribute) because the restarted
    /// flow ended in an error state instead of something to show.
    pub const DISPLAY_REMOVED_RESTART: &str = "display.removed.restart";
    /// `<tonk-display>` replaced content it had kept through a restart with
    /// what the restarted flow shows, with no empty moment between them.
    pub const DISPLAY_REPLACED_RESTART: &str = "display.replaced.restart";
    /// `<tonk-display>` removed rendered content to show an error.
    pub const DISPLAY_REMOVED_ERROR: &str = "display.removed.error";
    /// `<tonk-display>` removed rendered content because a model frame
    /// arrived and the downstream flow is rebuilt from it.
    pub const DISPLAY_REMOVED_MODEL: &str = "display.removed.model";
    /// `<tonk-display>` removed rendered content because its name resolved
    /// to a different entity.
    pub const DISPLAY_REMOVED_NAME: &str = "display.removed.name";
    /// `<tonk-display>` dropped a mounted view because a view frame came
    /// back without the facet it shows.
    pub const DISPLAY_REMOVED_VIEW_ABSENT: &str = "display.removed.view_absent";
    /// `<tonk-display>` applied an empty entity frame over rows it had
    /// rendered.
    pub const DISPLAY_ENTITY_EMPTIED: &str = "display.entity.emptied";
    /// `<tonk-display>` received rows again after an empty frame had emptied
    /// what it rendered: content deleted and then inserted again.
    pub const DISPLAY_ENTITY_REFILLED: &str = "display.entity.refilled";
    /// `<tonk-display>` held an empty reconnect frame over rendered rows
    /// instead of applying it.
    pub const DISPLAY_ENTITY_HELD: &str = "display.entity.held";

    /// The worker waited for the state lock's read guard (milliseconds).
    pub const WORKER_LOCK_READ_WAIT: &str = "worker.lock.read.wait";
    /// The worker waited for the state lock's write guard (milliseconds).
    pub const WORKER_LOCK_WRITE_WAIT: &str = "worker.lock.write.wait";
    /// How long a state lock read guard was held (milliseconds).
    pub const WORKER_LOCK_READ_HOLD: &str = "worker.lock.read.hold";
    /// How long a state lock write guard was held (milliseconds).
    pub const WORKER_LOCK_WRITE_HOLD: &str = "worker.lock.write.hold";
}

/// Summary of every value recorded under one duration name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Duration {
    /// How many values were recorded.
    pub count: u64,
    /// Their sum, in milliseconds.
    pub total_ms: f64,
    /// The largest one, in milliseconds.
    pub max_ms: f64,
}

/// Everything one registry has recorded since it started or was last reset.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// A random identifier minted when this registry started. A new value
    /// between two reads of the same realm means it was restarted: for the
    /// service worker, that the browser stopped and started it again.
    pub instance: String,
    /// Milliseconds since the Unix epoch when this registry started.
    pub started_at_ms: f64,
    /// Event counts by name.
    pub counters: BTreeMap<String, u64>,
    /// Durations by name.
    pub durations: BTreeMap<String, Duration>,
    /// Durations by name and the source location that produced them, as
    /// `name@file:line`. Kept apart from [`Snapshot::durations`] so the
    /// totals stay keyed on stable names while the sites say where to look.
    pub sites: BTreeMap<String, Duration>,
}

struct Registry {
    instance: String,
    started_at_ms: f64,
    counters: BTreeMap<&'static str, u64>,
    durations: BTreeMap<&'static str, Duration>,
    sites: BTreeMap<(&'static str, &'static str, u32), Duration>,
}

impl Registry {
    fn new() -> Self {
        Self {
            instance: mint_instance(),
            started_at_ms: now_epoch_ms(),
            counters: BTreeMap::new(),
            durations: BTreeMap::new(),
            sites: BTreeMap::new(),
        }
    }
}

// One registry per realm. A browser realm is single-threaded, so the web
// keeps it in a thread-local. A native process runs tasks on many threads
// (tokio's multi-threaded runtime), and a thread-local there would split
// one worker's numbers across threads and answer a read with one thread's
// share, so native keeps a single process-wide registry behind a mutex.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::new());
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn with_registry<R>(f: impl FnOnce(&mut Registry) -> R) -> R {
    REGISTRY.with(|registry| f(&mut registry.borrow_mut()))
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
static REGISTRY: std::sync::LazyLock<std::sync::Mutex<Registry>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Registry::new()));

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn with_registry<R>(f: impl FnOnce(&mut Registry) -> R) -> R {
    // A panic while recording leaves only counts behind, never a broken
    // invariant, so a poisoned lock is still safe to keep using.
    let mut registry = REGISTRY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    f(&mut registry)
}

/// Count one occurrence of `name`.
pub fn count(name: &'static str) {
    add(name, 1);
}

/// Count `n` occurrences of `name`.
pub fn add(name: &'static str, n: u64) {
    with_registry(|registry| {
        *registry.counters.entry(name).or_default() += n;
    });
}

/// Record one duration of `ms` milliseconds under `name`.
pub fn record(name: &'static str, ms: f64) {
    with_registry(|registry| {
        accumulate(registry.durations.entry(name).or_default(), ms);
    });
}

/// Record one duration under `name`, and under `name` at the source location
/// `site`, so a slow or contended path can be found from the snapshot alone.
pub fn record_at(name: &'static str, site: &'static std::panic::Location<'static>, ms: f64) {
    with_registry(|registry| {
        accumulate(registry.durations.entry(name).or_default(), ms);
        let key = (name, site.file(), site.line());
        accumulate(registry.sites.entry(key).or_default(), ms);
    });
}

fn accumulate(duration: &mut Duration, ms: f64) {
    duration.count += 1;
    duration.total_ms += ms;
    if ms > duration.max_ms {
        duration.max_ms = ms;
    }
}

/// Everything recorded since this registry started or was last [`reset`].
pub fn snapshot() -> Snapshot {
    with_registry(|registry| Snapshot {
        instance: registry.instance.clone(),
        started_at_ms: registry.started_at_ms,
        counters: registry
            .counters
            .iter()
            .map(|(name, value)| ((*name).to_owned(), *value))
            .collect(),
        durations: registry
            .durations
            .iter()
            .map(|(name, value)| ((*name).to_owned(), *value))
            .collect(),
        sites: registry
            .sites
            .iter()
            .map(|((name, file, line), value)| (format!("{name}@{file}:{line}"), *value))
            .collect(),
    })
}

/// Clear every counter and duration. The instance identifier and start time
/// stay, so a reset is not mistaken for a restart.
pub fn reset() {
    with_registry(|registry| {
        registry.counters.clear();
        registry.durations.clear();
        registry.sites.clear();
    });
}

/// A monotonic clock reading in milliseconds, for measuring a duration to
/// pass to [`record`].
pub fn now_ms() -> f64 {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        thread_local! {
            static ORIGIN: web_time::Instant = web_time::Instant::now();
        }
        ORIGIN.with(|origin| origin.elapsed().as_secs_f64() * 1000.0)
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        thread_local! {
            static ORIGIN: std::time::Instant = std::time::Instant::now();
        }
        ORIGIN.with(|origin| origin.elapsed().as_secs_f64() * 1000.0)
    }
}

fn now_epoch_ms() -> f64 {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        js_sys::Date::now()
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs_f64() * 1000.0)
            .unwrap_or_default()
    }
}

fn mint_instance() -> String {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let high = (js_sys::Math::random() * f64::from(u32::MAX)) as u32;
        let low = (js_sys::Math::random() * f64::from(u32::MAX)) as u32;
        format!("{high:08x}{low:08x}")
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        use std::hash::{BuildHasher, RandomState};
        format!("{:016x}", RandomState::new().hash_one(std::process::id()))
    }
}

/// Expose this realm's registry to its scripts as
/// `globalThis.__tonkDiagnostics`, with `snapshot()` and `reset()`.
/// Idempotent: a second call replaces the first's functions with equivalent
/// ones.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub fn install_global() {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::prelude::Closure;

    let hook = js_sys::Object::new();
    let read = Closure::<dyn Fn() -> wasm_bindgen::JsValue>::new(|| {
        let json = serde_json::to_string(&snapshot()).unwrap_or_else(|_| "{}".to_owned());
        js_sys::JSON::parse(&json).unwrap_or(wasm_bindgen::JsValue::NULL)
    });
    let clear = Closure::<dyn Fn()>::new(reset);
    let _ = js_sys::Reflect::set(&hook, &"snapshot".into(), read.as_ref().unchecked_ref());
    let _ = js_sys::Reflect::set(&hook, &"reset".into(), clear.as_ref().unchecked_ref());
    read.forget();
    clear.forget();
    let _ = js_sys::Reflect::set(&js_sys::global(), &"__tonkDiagnostics".into(), &hook);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registry is process-wide on native and tests run in parallel
    /// threads, so the tests that reset it take turns.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn it_counts_and_resets_without_changing_the_instance() {
        let _serial = serial();
        reset();
        let before = snapshot();
        count(names::SITE_GUEST_BOOT_MOUNT);
        add(names::SITE_GUEST_BOOT_MOUNT, 2);
        record(names::WORKER_LOCK_READ_WAIT, 3.0);
        record(names::WORKER_LOCK_READ_WAIT, 5.0);

        let after = snapshot();
        assert_eq!(after.counters[names::SITE_GUEST_BOOT_MOUNT], 3);
        let wait = after.durations[names::WORKER_LOCK_READ_WAIT];
        assert_eq!(wait.count, 2);
        assert_eq!(wait.total_ms, 8.0);
        assert_eq!(wait.max_ms, 5.0);

        reset();
        let cleared = snapshot();
        assert!(cleared.counters.is_empty());
        assert!(cleared.durations.is_empty());
        assert_eq!(cleared.instance, before.instance);
    }

    #[test]
    fn it_sees_what_another_thread_recorded() {
        let _serial = serial();
        reset();
        std::thread::spawn(|| count(names::SITE_GUEST_BOOT_REMOUNT))
            .join()
            .expect("counting thread");
        assert_eq!(snapshot().counters[names::SITE_GUEST_BOOT_REMOUNT], 1);
    }

    #[test]
    fn it_keys_sites_by_name_and_location() {
        let _serial = serial();
        reset();
        let here = std::panic::Location::caller();
        record_at(names::WORKER_LOCK_WRITE_HOLD, here, 7.0);
        let snapshot = snapshot();
        let key = format!(
            "{}@{}:{}",
            names::WORKER_LOCK_WRITE_HOLD,
            here.file(),
            here.line()
        );
        assert_eq!(snapshot.sites[&key].count, 1);
        assert_eq!(snapshot.durations[names::WORKER_LOCK_WRITE_HOLD].count, 1);
    }
}
