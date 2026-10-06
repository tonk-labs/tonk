//! Signing this browser in through another deployment: the browser's
//! `tonk account login --via`.
//!
//! An account lives where its passkey does, and a passkey is pinned to the
//! origin it was made on, so a browser on another deployment cannot open
//! the account itself. It asks the account's deployment for the grant a
//! terminal gets. [`SignInVia`] sends the page to
//! `<via>/settings/link?audience=<this profile>&callback=<here>`, the
//! person approves there with their passkey, and that deployment sends the
//! page back to the callback with an `account -> device` grant in the
//! fragment. [`FinishSignInVia`] installs the grant as this profile's root
//! and attaches the account where the grant's signed meta says it syncs,
//! which is the approving deployment's service, not this one.
//!
//! A native host has no https page to come back to, so its callback is a
//! one-shot loopback listener instead, as the CLI's is (see `loopback`).
//!
//! The callback names a one-shot request this profile recorded before it
//! left. An answer this browser did not ask for, such as someone else's
//! account handed in through a crafted link, matches no request and is
//! refused before anything is installed.

use serde::{Deserialize, Serialize};
use tonk_common::log;
use tonk_schema::command::{FinishSignInVia, SignInVia};
use tonk_schema::{ceremony, ceremony_state};
use url::Url;

use crate::worker::TonkState;

#[cfg(not(target_arch = "wasm32"))]
mod loopback;

/// Credential-store site holding the request this profile is waiting on.
const REQUEST_SITE: &str = "tonk-sign-in-via-v1";

/// How long an approval may take. An answer arriving later is refused,
/// so a request left behind by an abandoned attempt cannot be answered
/// days afterwards.
const REQUEST_TTL_SECONDS: u64 = 30 * 60;

/// The request this profile sent out and is waiting on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PendingRequest {
    /// The random id the callback carries back.
    request: String,
    /// The origin of the deployment asked.
    via: String,
    /// When the request went out, in seconds since the epoch.
    asked_at: u64,
}

impl PendingRequest {
    /// Whether an answer carrying `request` at `now` answers this request.
    fn answers(&self, request: &str, now: u64) -> bool {
        self.request == request && now.saturating_sub(self.asked_at) <= REQUEST_TTL_SECONDS
    }
}

/// What came back on the callback.
#[derive(Debug, PartialEq, Eq)]
struct Answer {
    /// The request id the callback carries.
    request: String,
    /// The approval or the refusal.
    outcome: Outcome,
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// The encoded authorization, as delivered in `#authorize=`.
    Granted(String),
    /// Why the other deployment declined, as delivered in `#deny=`.
    Denied(String),
}

/// The authorization the approving deployment delivers: the same payload
/// a terminal receives on its loopback callback.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Delivered {
    /// The `account -> device` grant, hex encoded.
    delegation_hex: String,
    /// Where the account syncs, for grants minted before the address rode
    /// the grant's signed meta. The meta wins when both are present.
    #[serde(default)]
    remote: String,
    /// What the approving deployment calls the credential that approved.
    #[serde(default)]
    credential_id: String,
}

/// The `<via>/settings/link` address that asks `via` to approve this
/// profile, with `here`'s own `/settings/link` as the callback.
///
/// Only an `https` deployment can be asked: the grant comes back in a URL,
/// and the approving page refuses plain `http` callbacks for the same
/// reason. `via` may be any address on the deployment; its origin is
/// what is asked.
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
fn approval_url(via: &str, here: &Url, audience: &str, request: &str) -> Result<String, String> {
    let origin = deployment(via)?;
    let callback = page_callback(here, &origin, request)?;
    let name = here.host_str().unwrap_or("a tonk deployment");
    ask(&origin, &callback, name, audience)
}

/// The origin of the deployment `via` names, which must be https.
fn deployment(via: &str) -> Result<String, String> {
    let via = Url::parse(via.trim())
        .map_err(|_| format!("{via} is not an address a deployment can be asked at"))?;
    if via.scheme() != "https" || via.host_str().is_none() {
        return Err(format!("{via} is not an https deployment"));
    }
    Ok(via.origin().ascii_serialization())
}

/// A page's callback: its own `/settings/link`, which reads the answer
/// and asserts [`FinishSignInVia`].
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
fn page_callback(here: &Url, via: &str, request: &str) -> Result<String, String> {
    Url::parse_with_params(
        here.join("settings/link")
            .map_err(|error| format!("this deployment's address is unusable: {error}"))?
            .as_str(),
        &[("via", via), ("request", request)],
    )
    .map(String::from)
    .map_err(|error| format!("the callback address did not build: {error}"))
}

/// The address asking the deployment at `origin` to approve `audience`,
/// answering at `callback`. `name` is what the approving page calls the
/// one asking.
fn ask(origin: &str, callback: &str, name: &str, audience: &str) -> Result<String, String> {
    let approval = Url::parse_with_params(
        &format!("{origin}/settings/link"),
        &[("audience", audience), ("callback", callback), ("name", name)],
    )
    .map_err(|error| format!("the approval address did not build: {error}"))?;
    Ok(approval.into())
}

/// Where the answer to `request` comes back, and what the approving page
/// calls this profile.
///
/// A page in a browser answers on its own `/settings/link`. A native host
/// has no https page to come back to, and the approving deployment only
/// delivers to an https page or a bare loopback address, so it answers
/// the way the `tonk` CLI does: on a one-shot loopback listener that
/// finishes the sign-in itself (see [`loopback`]).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn callback(
    _state: &super::AppState,
    request: &str,
    via: &str,
) -> Result<(String, String), String> {
    let here = super::customer::service_origin().map_err(|error| error.to_string())?;
    let callback = page_callback(&here, via, request)?;
    let name = here.host_str().unwrap_or("a tonk deployment").to_owned();
    Ok((callback, name))
}

/// See the browser's [`callback`].
#[cfg(not(target_arch = "wasm32"))]
async fn callback(
    state: &super::AppState,
    request: &str,
    via: &str,
) -> Result<(String, String), String> {
    let callback = loopback::listen(state.clone(), request.to_owned(), via.to_owned()).await?;
    Ok((callback, "tonk desktop".to_owned()))
}

/// Read the answer off the callback address: the request id from its
/// query, the grant or the refusal from its fragment.
fn parse_answer(url: &str) -> Result<Answer, String> {
    let url = Url::parse(url).map_err(|_| "the answer address is not a URL".to_string())?;
    let request = url
        .query_pairs()
        .find(|(key, _)| key == "request")
        .map(|(_, value)| value.into_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "the answer names no request".to_string())?;
    let fields: Vec<(String, String)> =
        url::form_urlencoded::parse(url.fragment().unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    let field = |name: &str| {
        fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };
    let outcome = match (field("authorize"), field("deny")) {
        (Some(authorization), _) if !authorization.is_empty() => Outcome::Granted(authorization),
        (_, Some(reason)) => Outcome::Denied(reason),
        _ => return Err("the answer carries neither a grant nor a refusal".to_string()),
    };
    Ok(Answer { request, outcome })
}

/// Decode the delivered authorization: base64 over the JSON payload.
fn decode_delivery(encoded: &str) -> Result<Delivered, String> {
    use base64::Engine as _;
    let payload = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| "the delivered authorization is not base64".to_string())?;
    serde_json::from_slice(&payload)
        .map_err(|error| format!("the delivered authorization is not readable: {error}"))
}

fn now() -> u64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn request_site(state: &TonkState) -> String {
    crate::credential::branch_site(REQUEST_SITE, &state.active_branch)
}

async fn save_pending(state: &TonkState, pending: &PendingRequest) -> Result<(), String> {
    let bytes = serde_json::to_vec(pending)
        .map_err(|error| format!("the sign-in request did not serialize: {error}"))?;
    state
        .profile
        .secrets()
        .site(request_site(state).as_str())
        .save(bytes)
        .perform(&state.profile)
        .await
        .map_err(|error| format!("the sign-in request was not recorded: {error}"))
}

async fn load_pending(state: &TonkState) -> Result<Option<PendingRequest>, String> {
    match state
        .profile
        .secrets()
        .site(request_site(state).as_str())
        .load::<Vec<u8>>()
        .perform(&state.profile)
        .await
    {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| format!("the recorded sign-in request is unreadable: {error}")),
        Err(error) if crate::credential::is_missing(&error) => Ok(None),
        Err(error) => Err(format!("the sign-in request could not be read: {error}")),
    }
}

async fn forget_pending(state: &TonkState) {
    if let Err(error) = state
        .profile
        .secrets()
        .site(request_site(state).as_str())
        .retract()
        .perform(&state.profile)
        .await
    {
        log!("sign-in-via: the answered request was not cleared: {error}");
    }
}

/// Record a fresh request to `via` and answer the address to send the
/// page to.
async fn start(app: &super::AppState, state: &TonkState, via: &str) -> Result<String, String> {
    let origin = deployment(via)?;
    let request = hex::encode(rand::random::<[u8; 16]>());
    let (callback, name) = callback(app, &request, &origin).await?;
    let approval = ask(&origin, &callback, &name, state.profile.did().as_ref())?;
    save_pending(
        state,
        &PendingRequest {
            request,
            via: origin,
            asked_at: now(),
        },
    )
    .await?;
    Ok(approval)
}

/// Check `url` answers the request this profile is waiting on, and read
/// the grant it delivers. A matching answer spends the request, granted or
/// declined, so one request yields one grant at most; an answer that
/// matches nothing leaves it waiting for the real one.
async fn answer(state: &TonkState, url: &str) -> Result<Delivered, String> {
    let answer = parse_answer(url)?;
    let pending = load_pending(state).await?.ok_or_else(|| {
        "this browser is not waiting on a sign-in; start again from the sign-in page".to_string()
    })?;
    if !pending.answers(&answer.request, now()) {
        return Err(
            "this answer is not for the sign-in this browser asked for, or it came too late; \
             start again from the sign-in page"
                .to_string(),
        );
    }
    forget_pending(state).await;
    match answer.outcome {
        Outcome::Granted(encoded) => decode_delivery(&encoded),
        Outcome::Denied(reason) => Err(format!("{} did not approve: {reason}", pending.via)),
    }
}

/// Install the delivered grant as this profile's root and attach the
/// account where the grant says it syncs. Local only: what reaches the
/// network comes after, in [`finish`].
///
/// Validate before selecting a branch, using the same checks as every
/// root: one proof, addressed to this profile, open in subject and command,
/// signed by the account it names. Keep the selected branch pinned through
/// installation and link completion.
async fn adopt(
    state: &super::AppState,
    source: Option<&super::ClientId>,
    delivered: Delivered,
) -> Result<super::profiles::AccountProfileGuard, String> {
    let bytes = hex::decode(delivered.delegation_hex.trim())
        .map_err(|_| "the delivered grant is not hex".to_string())?;
    let chain = {
        let tonk = state.read().await;
        super::identity::validate_grant(bytes.clone(), &tonk.profile.did())
            .await
            .map_err(|error| format!("the delivered grant is invalid: {error}"))?
    };
    let remote = tonk_invite::home_address(&chain)
        .map_err(|error| format!("the grant names an unusable sync address: {error:#}"))?
        .map(String::from)
        .or_else(|| Some(delivered.remote.trim().to_owned()).filter(|value| !value.is_empty()))
        .ok_or_else(|| "the grant names no sync address".to_string())?;
    let root_did = chain.issuer().to_string();
    let credential_id = Some(delivered.credential_id.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| root_did.clone());
    let delegation_hex = hex::encode(&bytes);
    let tonk = super::profiles::for_account(state.clone(), chain.issuer(), source)
        .await
        .map_err(|error| format!("the account profile could not be selected: {error}"))?;
    log!(
        "sign-in-via: account profile disposition {:?}",
        tonk.disposition()
    );
    super::identity::persist_root(
        &tonk,
        tonk_worker_api::SaveRootRequest {
            credential_id: credential_id.clone(),
            delegation_hex: delegation_hex.clone(),
            passkey: None,
            encryption_key: None,
        },
    )
    .await
    .map_err(|error| format!("the grant was not installed: {error}"))?;
    super::account::persist_link(
        &tonk,
        &tonk_worker_api::AccountLinkRequest {
            provider: String::new(),
            root_did,
            credential_id,
            delegation_hex,
            remote,
            initialize_name: false,
        },
    )
    .await
    .map_err(|error| format!("the account was not attached: {error}"))?;
    Ok(tonk)
}

/// Install the answer on `url`, then bring the account down the way a
/// passkey sign-in does once its root is recorded.
async fn finish(
    state: &super::AppState,
    source: Option<&super::ClientId>,
    url: &str,
) -> Result<(), String> {
    let delivered = {
        let tonk = state.read().await;
        answer(&tonk, url).await?
    };
    let tonk = adopt(state, source, delivered).await?;
    let state = &*tonk;
    super::account::finish_link(state)
        .await
        .map_err(|error| format!("the account did not finish linking: {error}"))?;
    // What was made while signed out joins the account signed back in to.
    if let Some(signed_out) = tonk.signed_out() {
        super::rotation::carry_from(state, signed_out).await;
    }
    if let Err(error) = super::account_state::push_account_main(state).await {
        log!("sign-in-via: the push behind the link did not land: {error}");
    }
    Ok(())
}

/// Run `tonk:sign-in-via`: record the request and send the page to the
/// deployment holding the account. A refusal is reported, not sent.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<SignInVia> for super::CommandEnv {
    async fn execute(&self, command: SignInVia) {
        let tonk = self.state().read().await;
        match start(self.state(), &tonk, &command.via.0).await {
            Ok(approval) => super::navigate::notify_navigate(self.client(), &approval),
            Err(error) => {
                super::ceremony::report(
                    &tonk,
                    ceremony::SIGN_IN_VIA,
                    ceremony_state::REFUSED,
                    &error,
                )
                .await
            }
        }
    }
}

/// Run `tonk:finish-sign-in-via`: install what came back and go home.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<FinishSignInVia> for super::CommandEnv {
    async fn execute(&self, command: FinishSignInVia) {
        complete(self.state(), self.client(), &command.url.0).await;
    }
}

/// Install the answer on `url`, report how it went, and go home.
async fn complete(state: &super::AppState, source: Option<&super::ClientId>, url: &str) {
    {
        let tonk = state.read().await;
        super::ceremony::report(&tonk, ceremony::SIGN_IN_VIA, ceremony_state::WORKING, "").await;
    }
    let result = finish(state, source, url).await;
    let tonk = state.read().await;
    match result {
        Ok(()) => {
            super::ceremony::report(&tonk, ceremony::SIGN_IN_VIA, ceremony_state::DONE, "").await;
            // A load, not a route change: signing back in can switch the
            // page onto the branch the account kept, and the page that
            // asked is left out of the reload every other tab gets (see
            // `profiles::promote`), so a route change would leave it bound
            // to the branch it started on and refused. The callback, with
            // the grant in its fragment, leaves the history too.
            super::navigate::notify_replace(source, "/");
        }
        Err(error) => {
            super::ceremony::report(&tonk, ceremony::SIGN_IN_VIA, ceremony_state::FAILED, &error)
                .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_service_worker);

    const DEVICE: &str = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

    fn here() -> Url {
        "https://tonk.host/".parse().unwrap()
    }

    #[dialog_common::test]
    fn it_asks_the_deployment_for_this_profile_with_a_callback_here() {
        let approval =
            approval_url("https://tonk.network/anything?x=1", &here(), DEVICE, "r1").unwrap();
        let approval = Url::parse(&approval).unwrap();

        assert_eq!(
            approval.origin().ascii_serialization(),
            "https://tonk.network"
        );
        assert_eq!(approval.path(), "/settings/link");
        let params: std::collections::BTreeMap<_, _> =
            approval.query_pairs().into_owned().collect();
        assert_eq!(params["audience"], DEVICE);
        assert_eq!(params["name"], "tonk.host");
        assert_eq!(
            params["callback"],
            "https://tonk.host/settings/link?via=https%3A%2F%2Ftonk.network&request=r1"
        );
        assert!(
            tonk_worker_api::callback::delivery_url(&params["callback"], &[]).is_ok(),
            "the approving deployment must accept the callback it is handed"
        );
    }

    #[dialog_common::test]
    fn it_refuses_to_ask_a_deployment_that_is_not_https() {
        for via in [
            "http://tonk.network",
            "tonk.network",
            "javascript:alert(1)",
            "",
        ] {
            assert!(
                approval_url(via, &here(), DEVICE, "r1").is_err(),
                "a grant must not be asked for over {via:?}"
            );
        }
    }

    #[dialog_common::test]
    fn it_reads_a_grant_or_a_refusal_off_the_callback() {
        let delivered = tonk_worker_api::callback::delivery_url(
            "https://tonk.host/settings/link?via=https%3A%2F%2Ftonk.network&request=r1",
            &[
                ("authorize", "eyJhIjoxfQ=="),
                ("redirect", "https://tonk.network/settings"),
            ],
        )
        .unwrap();
        assert_eq!(
            parse_answer(&delivered).unwrap(),
            Answer {
                request: "r1".into(),
                outcome: Outcome::Granted("eyJhIjoxfQ==".into()),
            }
        );

        let declined = tonk_worker_api::callback::delivery_url(
            "https://tonk.host/settings/link?via=https%3A%2F%2Ftonk.network&request=r2",
            &[("deny", "declined in the browser")],
        )
        .unwrap();
        assert_eq!(
            parse_answer(&declined).unwrap(),
            Answer {
                request: "r2".into(),
                outcome: Outcome::Denied("declined in the browser".into()),
            }
        );
    }

    #[dialog_common::test]
    fn it_refuses_an_answer_without_a_request_or_an_outcome() {
        for url in [
            "https://tonk.host/settings/link?via=x#authorize=abc",
            "https://tonk.host/settings/link?via=x&request=#authorize=abc",
            "https://tonk.host/settings/link?via=x&request=r1",
            "https://tonk.host/settings/link?via=x&request=r1#authorize=",
            "not a url",
        ] {
            assert!(
                parse_answer(url).is_err(),
                "{url} must not read as an answer"
            );
        }
    }

    #[dialog_common::test]
    fn it_answers_only_its_own_request_and_only_in_time() {
        let pending = PendingRequest {
            request: "r1".into(),
            via: "https://tonk.network".into(),
            asked_at: 1_000,
        };
        assert!(pending.answers("r1", 1_000 + 60));
        assert!(!pending.answers("r2", 1_000 + 60), "another request's id");
        assert!(
            !pending.answers("r1", 1_000 + REQUEST_TTL_SECONDS + 1),
            "an answer arriving after the request lapsed"
        );
    }

    #[dialog_common::test]
    fn it_decodes_the_payload_the_approving_page_delivers() {
        use base64::Engine as _;
        let payload = serde_json::json!({
            "delegationHex": "00ff",
            "remote": "https://tonk.network/ucan/",
            "credentialId": "did:key:zRoot",
            "attachmentId": "bafy",
        })
        .to_string();
        let encoded = base64::engine::general_purpose::STANDARD.encode(payload);
        let delivered = decode_delivery(&encoded).unwrap();
        assert_eq!(delivered.delegation_hex, "00ff");
        assert_eq!(delivered.remote, "https://tonk.network/ucan/");
        assert_eq!(delivered.credential_id, "did:key:zRoot");
        assert!(decode_delivery("not base64!").is_err());
    }
}

/// The request binding and the install, against a real profile store.
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod store_tests {
    use super::*;
    use dialog_credentials::Ed25519Signer;
    use dialog_varsig::Principal as _;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    use crate::router::tests::test_state_without_root;

    const HOME: &str = "https://tonk.network/ucan/";

    /// What `tonk.network` delivers after approving `device`: the payload
    /// `authorize_device` builds, base64 encoded as it rides the fragment.
    async fn delivery_for(root_seed: u8, device: &dialog_varsig::Did) -> (String, String) {
        use base64::Engine as _;
        let root = Ed25519Signer::import(&[root_seed; 32]).await.unwrap();
        let root_did = root.did().to_string();
        let authorized = tonk_identity::ceremony::authorize_device(root, device.clone(), HOME)
            .await
            .unwrap();
        let payload = serde_json::json!({
            "delegationHex": authorized.delegation_hex,
            "remote": HOME,
            "credentialId": authorized.root_did,
            "attachmentId": "",
        })
        .to_string();
        (
            base64::engine::general_purpose::STANDARD.encode(payload),
            root_did,
        )
    }

    fn callback(request: &str, encoded: &str) -> String {
        tonk_worker_api::callback::delivery_url(
            &format!(
                "https://tonk.host/settings/link?via=https%3A%2F%2Ftonk.network&request={request}"
            ),
            &[("authorize", encoded)],
        )
        .unwrap()
    }

    async fn waiting_on(state: &TonkState, request: &str) {
        save_pending(
            state,
            &PendingRequest {
                request: request.into(),
                via: "https://tonk.network".into(),
                asked_at: now(),
            },
        )
        .await
        .unwrap();
    }

    #[dialog_common::test]
    async fn it_installs_the_grant_it_asked_for_and_attaches_the_home_it_names() {
        let state = test_state_without_root().await;
        let (encoded, root_did) = delivery_for(71, &state.profile.did()).await;
        waiting_on(&state, "r1").await;

        let delivered = answer(&state, &callback("r1", &encoded)).await.unwrap();
        let app_state = std::sync::Arc::new(tokio::sync::RwLock::new(state));
        let state = adopt(&app_state, None, delivered).await.unwrap();

        let root = super::super::identity::local_root(&state).await.unwrap();
        assert_eq!(root.root_did.to_string(), root_did);
        assert_eq!(
            super::super::account::provider(&state).await.as_deref(),
            Some(HOME),
            "the account syncs where the grant says, not with this deployment"
        );
        assert!(
            load_pending(&state).await.unwrap().is_none(),
            "the request is spent"
        );
    }

    /// The local branch record written when the initial link finishes.
    /// Record it directly so the directory is never fetched from a remote.
    async fn record_linked_branch(state: &TonkState) {
        let root = super::super::identity::root_did(state).await.unwrap();
        let address =
            dialog_repository::SiteAddress::from(dialog_remote_ucan::UcanAddress::new(HOME));
        super::super::account_state::record_account_branch(state, &root, &address).await;
    }

    #[dialog_common::test]
    async fn it_restores_the_retained_account_branch_and_local_directory_on_relogin() {
        let (app, state, _lsp) = crate::api_router_with_state(test_state_without_root().await);
        let space = crate::router::tests::put_repo(&app, "unsynced-space").await;
        let (encoded, _) = delivery_for(75, &state.read().await.profile.did()).await;
        let original = state.read().await.active_branch.clone();
        let selected = adopt(&state, None, decode_delivery(&encoded).unwrap())
            .await
            .unwrap();
        assert_eq!(selected.active_branch, original);
        record_linked_branch(&selected).await;
        drop(selected);

        for request in ["return-1", "return-2"] {
            super::super::profiles::sign_out(&state, None)
                .await
                .unwrap();
            let before = {
                let tonk = state.read().await;
                assert_ne!(tonk.active_branch, original);
                waiting_on(&tonk, request).await;
                super::super::profile::local_branches(&tonk).await.len()
            };
            let delivered = {
                let tonk = state.read().await;
                answer(&tonk, &callback(request, &encoded)).await.unwrap()
            };
            let selected = adopt(&state, None, delivered).await.unwrap();
            assert_eq!(selected.active_branch, original);
            assert_eq!(
                super::super::profile::local_branches(&selected).await.len(),
                before,
                "re-login must not create another account branch"
            );
            drop(selected);
            let axum::Json(profile) =
                super::super::profile::get_profile(axum::extract::State(state.clone()))
                    .await
                    .unwrap();
            assert!(
                profile.space.iter().any(|entry| entry.key == space),
                "the restored session must include the retained local directory"
            );
        }
    }

    #[dialog_common::test]
    async fn it_refuses_an_answer_this_browser_did_not_ask_for() {
        let state = test_state_without_root().await;
        let (encoded, _) = delivery_for(72, &state.profile.did()).await;

        let unasked = answer(&state, &callback("r1", &encoded)).await;
        assert!(matches!(unasked, Err(ref error) if error.contains("not waiting on a sign-in")));

        waiting_on(&state, "r1").await;
        let crafted = answer(&state, &callback("someone-else", &encoded)).await;
        assert!(
            matches!(crafted, Err(ref error) if error.contains("not for the sign-in this browser asked for"))
        );
        assert!(
            super::super::identity::load_record(&state)
                .await
                .unwrap()
                .is_none(),
            "nothing was installed"
        );
        assert!(
            load_pending(&state).await.unwrap().is_some(),
            "a stray answer does not spend the request it failed to match"
        );
    }

    /// A browser signed in through another deployment keeps its spaces
    /// where its account syncs, so that deployment provisions them and
    /// only its refusal counts. `/provider/add` and `/provider/remove` go
    /// there, never to the deployment serving this page, which holds no
    /// customer for the account.
    #[dialog_common::test]
    async fn it_provisions_and_deprovisions_at_the_deployment_it_signed_in_through() {
        use tonk_schema::prelude::DidExt as _;

        let state = test_state_without_root().await;
        let (encoded, _) = delivery_for(74, &state.profile.did()).await;
        waiting_on(&state, "r1").await;
        let delivered = answer(&state, &callback("r1", &encoded)).await.unwrap();
        let app_state = std::sync::Arc::new(tokio::sync::RwLock::new(state));
        let state = adopt(&app_state, None, delivered).await.unwrap();

        // What the account's branch brings once it syncs in: the key its
        // first device published, which a new space's seed is sealed to.
        // This browser has no passkey to derive it from.
        let account = super::super::identity::root_did(&state).await.unwrap();
        let recipient =
            tonk_identity::envelope::AccountSecret::from_bytes(zeroize::Zeroizing::new([74; 32]))
                .secret()
                .did();
        state
            .reactor
            .profile_repository()
            .branch(&state.active_branch)
            .transaction()
            .assert(tonk_schema::AccountSealedInbox::new(
                account.this(),
                recipient.this(),
            ))
            .commit()
            .perform(&state.operator)
            .await
            .unwrap();

        assert!(super::super::repository::remote_is_own_service(&state, HOME).await);
        assert!(
            !super::super::repository::remote_is_own_service(&state, "https://tonk.host/ucan/")
                .await,
            "the deployment serving the page does not hold this account"
        );

        let space = super::super::repository::create_repository(
            &state,
            "Kept at home",
            &Default::default(),
        )
        .await
        .unwrap()
        .did();

        let calls = js_sys::Array::new();
        let _calls =
            crate::router::tests::GlobalPropertyGuard::replace("__tonkProviderCalls", &calls);
        let fetch = js_sys::Function::new_with_args(
            "request",
            "globalThis.__tonkProviderCalls.push(request.method + ' ' + request.url);
             return Promise.resolve(new Response(new Uint8Array(0), { status: 200 }));",
        );
        let _fetch = crate::router::tests::GlobalPropertyGuard::replace("fetch", fetch.as_ref());
        let sent = || {
            calls
                .iter()
                .map(|call| call.as_string().unwrap())
                .collect::<Vec<_>>()
        };

        super::super::repository::provision_space_consumer(&state, &space)
            .await
            .unwrap();
        assert_eq!(sent(), ["POST https://tonk.network/ucan/"]);
        assert!(super::super::customer::space_provider_recorded(&state, &space).await);

        // Through the command the Hub fires, which has no request to say
        // where the space was provided.
        drop(state);
        let state = app_state;
        let env =
            crate::router::CommandEnv::new(state.clone(), crate::router::CommandOrigin::default());
        <crate::router::CommandEnv as dialog_capability::Provider<
            tonk_schema::command::RemoveSpace,
        >>::execute(
            &env,
            tonk_schema::command::RemoveSpace {
                this: "cmd:remove-space".parse().expect("entity"),
                subject: tonk_schema::domain::command::current::remove_space::Subject(space.this()),
            },
        )
        .await;
        assert_eq!(
            sent(),
            [
                "POST https://tonk.network/ucan/",
                "POST https://tonk.network/ucan/"
            ]
        );
        assert!(
            !super::super::customer::space_provider_recorded(&*state.read().await, &space).await,
            "the home accepted the removal"
        );
    }

    #[dialog_common::test]
    async fn it_refuses_a_grant_addressed_to_another_device() {
        let app_state =
            std::sync::Arc::new(tokio::sync::RwLock::new(test_state_without_root().await));
        let device = app_state.read().await.profile.did();
        let (valid, _) = delivery_for(73, &device).await;
        let selected = adopt(&app_state, None, decode_delivery(&valid).unwrap())
            .await
            .unwrap();
        record_linked_branch(&selected).await;
        drop(selected);
        super::super::profiles::sign_out(&app_state, None)
            .await
            .unwrap();
        let elsewhere = Ed25519Signer::import(&[9; 32]).await.unwrap().did();
        let (encoded, _) = delivery_for(73, &elsewhere).await;
        let (delivered, before) = {
            let state = app_state.read().await;
            waiting_on(&state, "r1").await;
            (
                answer(&state, &callback("r1", &encoded)).await.unwrap(),
                state.active_branch.clone(),
            )
        };
        let installed = adopt(&app_state, None, delivered).await;
        let state = app_state.read().await;
        assert_eq!(
            state.active_branch, before,
            "an invalid grant must not select its issuer's retained branch"
        );
        assert!(
            matches!(installed, Err(ref error) if error.contains("audience is not the current profile"))
        );
        assert!(
            super::super::identity::load_record(&state)
                .await
                .unwrap()
                .is_none(),
            "nothing was installed"
        );
        assert!(super::super::account::provider(&state).await.is_none());
    }
}
