//! Sending a signed-out user to sign up, and finishing what they started.
//!
//! Durable authority is only ever issued to an account (see
//! `router::account::require_account`), so a signed-out user who asks to
//! create a spot or join an invite is refused by the service worker. The
//! refusal arrives here as an `account-required` message carrying the intent
//! that was refused.
//!
//! This module is the whole of what the page does with it: park the intent,
//! and go to `/account`. The account element runs the ceremony — email, code,
//! and the passkey created as part of it — and calls back into [`finish`] on
//! success, which replays the parked intent and lands the user where they were
//! going.
//!
//! There is no passkey modal here any more. The one that used to live here
//! offered "Create a new passkey" with nothing behind it: no address, no
//! recovery, no service. The credential it minted looked like an account to
//! everyone except the system that issued it, and the spots created against it
//! were local-only and never backed up.

use tonk_worker_api::{AccountRequired, JoinResponse, PendingIntent};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::MessageEvent;

use std::cell::Cell;

/// Where the intent waits while the user signs up.
///
/// `sessionStorage`, not memory: signing up is a real navigation to
/// `/account`, so nothing in this document survives it. Per-origin, per-tab,
/// and gone when the tab closes.
///
/// A `DurableJoin` intent carries an authority-bearing invite URL. That URL is
/// already in the address bar during a join and already stored by the worker
/// as the guest record, so parking it here for one navigation adds no exposure
/// this flow did not already have. [`PendingIntent`]'s `Debug` redacts it.
const PENDING_KEY: &str = "tonk.pending-intent";

/// The account route, and the parameter naming where to return to.
const ACCOUNT_PATH: &str = "/account";
const NEXT_PARAM: &str = "next";

thread_local! {
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
    /// Set once the gate has committed to leaving for `/account`. A second
    /// refusal arriving during that navigation has nowhere to go and must not
    /// overwrite the intent already parked.
    static LEAVING: Cell<bool> = const { Cell::new(false) };
}

fn session_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.session_storage().ok().flatten()
}

fn park(intent: &PendingIntent) {
    let Some(storage) = session_storage() else {
        return;
    };
    let Ok(payload) = serde_json::to_string(intent) else {
        return;
    };
    let _ = storage.set_item(PENDING_KEY, &payload);
}

/// Take the parked intent, if any. One shot: the entry is removed whether or
/// not it parses, so a payload this build cannot read cannot wedge the flow.
pub(crate) fn take_pending() -> Option<PendingIntent> {
    let storage = session_storage()?;
    let payload = storage.get_item(PENDING_KEY).ok().flatten()?;
    let _ = storage.remove_item(PENDING_KEY);
    serde_json::from_str(&payload).ok()
}

/// Forget any parked intent.
///
/// The account page calls this when it was opened directly rather than by the
/// gate — no `next`, so nothing sent the user here. Without it, an intent
/// abandoned earlier in this tab would replay on the next sign-in and create a
/// spot nobody asked for.
pub(crate) fn discard_pending() {
    if let Some(storage) = session_storage() {
        let _ = storage.remove_item(PENDING_KEY);
    }
}

/// This document's location as a host-relative path, for `next`.
fn here() -> String {
    let Some(location) = web_sys::window().map(|window| window.location()) else {
        return "/".to_owned();
    };
    let mut path = location.pathname().unwrap_or_else(|_| "/".to_owned());
    path.push_str(&location.search().unwrap_or_default());
    path.push_str(&location.hash().unwrap_or_default());
    path
}

/// Whether `next` may be navigated to.
///
/// Host-relative only. A leading `//` is protocol-relative — the browser reads
/// `//evil.test/x` as another origin — so the parameter would otherwise be an
/// open redirect off an ordinary-looking link.
pub(crate) fn is_safe_next(next: &str) -> bool {
    next.starts_with('/') && !next.starts_with("//")
}

/// The account URL that returns to `next` when it is done.
pub(crate) fn sign_in_url(next: &str) -> String {
    if !is_safe_next(next) {
        return ACCOUNT_PATH.to_owned();
    }
    let encoded: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair(NEXT_PARAM, next)
        .finish();
    format!("{ACCOUNT_PATH}?{encoded}")
}

/// The `next` this document was asked to return to, when it is safe.
pub(crate) fn requested_next() -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    let query = search.strip_prefix('?')?;
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(name, _)| name == NEXT_PARAM)
        .map(|(_, value)| value.into_owned())
        .filter(|next| is_safe_next(next))
}

/// Leave for the account page, parking `intent` to replay on the way back.
fn go_sign_in(intent: PendingIntent) {
    if LEAVING.with(|leaving| leaving.replace(true)) {
        return;
    }
    park(&intent);
    tonk_host::navigate_to(&sign_in_url(&here()));
}

/// Perform the operation the account gate interrupted.
///
/// Each arm navigates on success — into the space that was created, or the one
/// that was joined — so a replay ends where the user was trying to get to
/// rather than back where they were refused.
pub(crate) async fn replay(intent: PendingIntent) -> Result<(), String> {
    match intent {
        PendingIntent::CreateSpace {
            name,
            remote,
            revocation_url,
            template,
        } => {
            let created: tonk_worker_api::CreateSpaceResponse = post(
                "/api/spaces",
                &tonk_worker_api::CreateSpaceRequest {
                    name,
                    remote,
                    revocation_url,
                    template,
                },
            )
            .await?;
            tonk_host::navigate_to(&format!("/space/{}", created.key));
        }
        PendingIntent::DurableJoin { url } => {
            let joined: JoinResponse =
                post("/api/profile/join", &tonk_worker_api::JoinRequest { url }).await?;
            let repository = match joined {
                JoinResponse::Joined { repository } | JoinResponse::Renewed { repository } => {
                    repository
                }
            };
            tonk_host::navigate_to(&format!("/space/{}", repository.name));
        }
    }
    Ok(())
}

/// POST `body` to this origin's `path` and read the answer, refusing anything
/// but success — the shape both replays share.
async fn post<B: serde::Serialize, T: serde::de::DeserializeOwned>(
    path: &str,
    body: &B,
) -> Result<T, String> {
    let origin = web_sys::window()
        .and_then(|window| window.location().origin().ok())
        .ok_or_else(|| "window origin is unavailable".to_string())?;
    let response = reqwest::Client::new()
        .post(format!("{origin}{path}"))
        .json(body)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("operation failed with {}", response.status()));
    }
    response.json().await.map_err(|error| error.to_string())
}

/// Perform the parked intent, if the gate parked one.
///
/// Answers `Ok(true)` when it replayed — and therefore navigated. A replay
/// failure is reported rather than swallowed: the account is real either way,
/// but the operation the user asked for is not done, and saying "you're logged
/// in" and stopping there would be a lie.
pub(crate) async fn resume_pending() -> Result<bool, String> {
    match take_pending() {
        Some(intent) => replay(intent).await.map(|()| true),
        None => Ok(false),
    }
}

/// Finish a sign-in that just happened: replay what the gate interrupted, or
/// go back to wherever the user came from.
///
/// Only for the moment a ceremony completes. A page load that merely FINDS an
/// account uses [`resume_pending`] instead: `next` means "here is the way
/// back", and honouring it on load would bounce someone who opened their
/// account settings from a spot straight out of the page they asked for.
pub(crate) async fn finish() -> Result<bool, String> {
    if resume_pending().await? {
        return Ok(true);
    }
    match requested_next() {
        Some(next) => {
            tonk_host::navigate_to(&next);
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Install the top-document service-worker account-request listener.
pub fn install() {
    if INSTALLED.with(|installed| installed.replace(true)) {
        return;
    }
    let Some(window) = web_sys::window() else {
        return;
    };
    let service_worker = window.navigator().service_worker();
    let listener = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let Ok(message) = serde_wasm_bindgen::from_value::<AccountRequired>(event.data()) else {
            return;
        };
        if message.message_type != tonk_worker_api::ACCOUNT_REQUIRED {
            return;
        }
        go_sign_in(message.intent);
    });
    let _ = service_worker
        .add_event_listener_with_callback("message", listener.as_ref().unchecked_ref());
    listener.forget();
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_browser);

    /// `next` is a path this origin will navigate to, so it must not be able
    /// to name another one. A protocol-relative value is the whole trick: the
    /// browser reads `//evil.test/x` as an absolute URL.
    #[dialog_common::test]
    fn it_refuses_a_next_that_leaves_the_origin() {
        for safe in ["/", "/space/abc", "/join?x=1#seed"] {
            assert!(is_safe_next(safe), "{safe}");
        }
        for unsafe_next in [
            "//evil.test/x",
            "https://evil.test/x",
            "javascript:alert(1)",
            "space/abc",
            "",
        ] {
            assert!(!is_safe_next(unsafe_next), "{unsafe_next}");
        }
    }

    /// An unusable `next` drops out of the URL entirely rather than riding
    /// along to be re-validated later.
    #[dialog_common::test]
    fn it_builds_an_account_url_that_returns_where_it_came_from() {
        assert_eq!(
            sign_in_url("/space/did:key:z6Mk?a=1"),
            "/account?next=%2Fspace%2Fdid%3Akey%3Az6Mk%3Fa%3D1"
        );
        assert_eq!(sign_in_url("//evil.test/x"), "/account");
    }

    /// One shot, and unparseable payloads clear themselves — a stored intent
    /// this build cannot read must not wedge every later sign-in.
    #[dialog_common::test]
    fn it_takes_a_parked_intent_exactly_once() {
        let storage = session_storage().expect("session storage");
        let _ = storage.remove_item(PENDING_KEY);

        park(&PendingIntent::CreateSpace {
            name: "Notes".into(),
            remote: None,
            revocation_url: None,
            template: None,
        });
        assert!(matches!(
            take_pending(),
            Some(PendingIntent::CreateSpace { .. })
        ));
        assert!(take_pending().is_none(), "the intent is consumed");

        storage.set_item(PENDING_KEY, "{not json").unwrap();
        assert!(take_pending().is_none());
        assert!(
            storage.get_item(PENDING_KEY).unwrap().is_none(),
            "an unreadable payload is cleared, not left to fail forever"
        );
    }

    #[dialog_common::test]
    fn it_discards_a_parked_intent() {
        park(&PendingIntent::DurableJoin {
            url: "https://tonk.network/join#seed".into(),
        });
        discard_pending();
        assert!(take_pending().is_none());
    }
}
