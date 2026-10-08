//! The sweep that drops what the worker holds for clients that are gone.
//!
//! A reload gives a page a fresh client id, so whatever the worker keyed
//! by the old id (reactor subscribers, the site facts it stamped into
//! branch overlays, its view binding) would otherwise outlive the page.
//! [`sweep_stale_clients`] decides which clients are provably dead and
//! drops their state.

use std::collections::HashSet;

use js_sys::Array;
use tonk_common::log;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Client, ServiceWorkerGlobalScope};

use crate::router::{AppState, ClientId};

/// Drop the reactor state and the `ViewBindings` entry of every client
/// that is provably dead (see [`reap_dead_clients`]).
///
/// Best-effort: runs opportunistically from `on_fetch`, so an idle
/// worker may hold stale entries briefly.
pub async fn sweep_stale_clients(state: &AppState) {
    let global: ServiceWorkerGlobalScope = match js_sys::global().dyn_into() {
        Ok(g) => g,
        Err(_) => {
            log!("sweep: not in a service worker scope");
            return;
        }
    };
    let live_promise = global.clients().match_all();
    let live_value = match JsFuture::from(live_promise).await {
        Ok(v) => v,
        Err(e) => {
            log!("sweep: clients.matchAll failed: {e:?}");
            return;
        }
    };
    let live_ids: HashSet<String> = Array::from(&live_value)
        .iter()
        .filter_map(|v| v.dyn_into::<Client>().ok().map(|c| c.id()))
        .collect();

    // ONE verdict, used by every consumer below: which clients are provably
    // dead. See `reap_dead_clients` — bare absence from `matchAll()` is not
    // death, because a booting page (whose id is the navigation's
    // `resultingClientId`) is absent for its entire boot.
    let dead = reap_dead_clients(state, &live_ids).await;
    if dead.clients.is_empty() {
        return;
    }

    sweep_reactor_state(state, &dead).await;

    let views = state.read().await.view_bindings.clone();
    let mut views = views.write().await;
    let held = views.len();
    views.retain(|client, _| !dead.clients.contains(client));
    let dropped = held - views.len();
    if dropped > 0 {
        log!("sweep: dropped {dropped} view bindings");
    }
}

/// The clients this sweep proved dead, and the site entities they had
/// stamped (carried out of the ledger, since the site URI is not a
/// function of the client id — see [`crate::router::ClientState`]).
#[derive(Default)]
struct Dead {
    clients: HashSet<ClientId>,
    sites: HashSet<String>,
}

/// Reconcile the liveness ledger against this poll's live set and return
/// the clients that are **provably dead**: ones we previously observed
/// alive in `clients.matchAll()` that are now gone. Reaped entries are
/// removed from the ledger.
///
/// This is the ONLY place death is decided, so no two consumers of the
/// sweep can disagree about it.
///
/// Absence from `matchAll()` is not death on its own. A navigation's
/// client id is the `FetchEvent`'s `resultingClientId` — the id the
/// *future* document will get — so a booting page is legitimately absent
/// (with or without `includeUncontrolled`) for its whole boot, which is
/// exactly when it stamps its site and opens its subscriptions. Reaping
/// on bare absence deletes the live page's own state out from under it,
/// and its display then waits forever on a subscription nobody will ever
/// feed. So a client must be seen ALIVE once before its later absence
/// counts as death.
async fn reap_dead_clients(state: &AppState, live_ids: &HashSet<String>) -> Dead {
    let (clients, views) = {
        let snap = state.read().await;
        (snap.clients.clone(), snap.view_bindings.clone())
    };

    // Enroll every client the worker holds state for, not just those that
    // stamped a site: a view client registers a view binding but no site,
    // and a client absent from the ledger could never be judged dead — its
    // state would leak for the worker's lifetime. Enrolled as
    // NOT-seen-live, so enrollment alone can never condemn a client; only
    // a later poll that actually sees it alive can.
    let held: Vec<ClientId> = views.read().await.keys().cloned().collect();

    let mut ledger = clients.write().await;
    for client in held {
        ledger.entry(client).or_default();
    }

    let mut dead = Dead::default();
    ledger.retain(|client, entry| {
        if live_ids.contains(&client.0) {
            // Observed alive. From here on, absence means death.
            entry.seen_live = true;
            return true;
        }
        if !entry.seen_live {
            // Never yet seen alive: presumed booting, not dead. Keep.
            return true;
        }
        // Born, then died.
        dead.sites.extend(entry.sites.keys().cloned());
        dead.clients.insert(client.clone());
        false
    });
    dead
}

/// Drop dead clients' reactor state: their SSE subscribers and the site
/// facts they stamped into branch overlays.
///
/// Both accumulate without this. A reload assigns the page a fresh client
/// id, so the old id's subscriptions keep re-evaluating on every poll (a
/// vanished client's response stream may never cancel, so the
/// send-failure prune never fires) and its `site:` overlay facts grow the
/// fold that every read of the branch pays.
async fn sweep_reactor_state(state: &AppState, dead: &Dead) {
    let dead_ids: HashSet<&str> = dead.clients.iter().map(|c| c.0.as_str()).collect();
    let snap = state.read().await;

    for branch in snap.reactor.cached_branch_states() {
        branch.retain_subscribers(|client| !dead_ids.contains(client));
        // Deliberately NO `schedule_poll` here. This sweep runs from
        // `on_fetch` for EVERY request, including static assets — and those
        // have no dispatcher to drain the reactor's scheduled-poll set, so a
        // poll scheduled here would sit in `pending_polls` until some
        // unrelated request happened to drain it, with every sweep piling on
        // another. Nor is a poll needed: the facts removed belonged to a
        // client that is gone, so no live subscriber was reading them and
        // their removal changes nothing anyone can observe.
        branch.retain_overlay_entities(|entity| !dead.sites.contains(entity.as_str()));
    }

    log!(
        "sweep: reaped {} dead client(s), {} site(s)",
        dead.clients.len(),
        dead.sites.len()
    );
}
