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
fn approval_url(via: &str, here: &Url, audience: &str, request: &str) -> Result<String, String> {
    let via = Url::parse(via.trim())
        .map_err(|_| format!("{via} is not an address a deployment can be asked at"))?;
    if via.scheme() != "https" || via.host_str().is_none() {
        return Err(format!("{via} is not an https deployment"));
    }
    let origin = via.origin().ascii_serialization();
    let callback = Url::parse_with_params(
        here.join("settings/link")
            .map_err(|error| format!("this deployment's address is unusable: {error}"))?
            .as_str(),
        &[("via", origin.as_str()), ("request", request)],
    )
    .map_err(|error| format!("the callback address did not build: {error}"))?;
    let name = here.host_str().unwrap_or("a tonk deployment");
    let approval = Url::parse_with_params(
        &format!("{origin}/settings/link"),
        &[
            ("audience", audience),
            ("callback", callback.as_str()),
            ("name", name),
        ],
    )
    .map_err(|error| format!("the approval address did not build: {error}"))?;
    Ok(approval.into())
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
        .credential()
        .site(request_site(state).as_str())
        .save(bytes)
        .perform(&state.operator)
        .await
        .map_err(|error| format!("the sign-in request was not recorded: {error}"))
}

async fn load_pending(state: &TonkState) -> Result<Option<PendingRequest>, String> {
    match state
        .profile
        .credential()
        .site(request_site(state).as_str())
        .load::<Vec<u8>>()
        .perform(&state.operator)
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
        .credential()
        .site(request_site(state).as_str())
        .retract()
        .perform(&state.operator)
        .await
    {
        log!("sign-in-via: the answered request was not cleared: {error}");
    }
}

/// Record a fresh request to `via` and answer the address to send the
/// page to.
async fn start(state: &TonkState, via: &str) -> Result<String, String> {
    let here = super::customer::service_origin().map_err(|error| error.to_string())?;
    let request = hex::encode(rand::random::<[u8; 16]>());
    let approval = approval_url(via, &here, state.profile.did().as_ref(), &request)?;
    let origin = Url::parse(&approval)
        .map(|approval| approval.origin().ascii_serialization())
        .unwrap_or_default();
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
/// [`super::identity::persist_root`] checks the grant the way it checks
/// every root: one proof, addressed to this profile, open in subject and
/// command, signed by the account it names.
async fn adopt(state: &TonkState, delivered: Delivered) -> Result<(), String> {
    let bytes = hex::decode(delivered.delegation_hex.trim())
        .map_err(|_| "the delivered grant is not hex".to_string())?;
    let chain = dialog_ucan_core::DelegationChain::try_from(bytes.as_slice())
        .map_err(|error| format!("the delivered grant is not a delegation: {error:?}"))?;
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
    super::identity::persist_root(
        state,
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
        state,
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
    .map_err(|error| format!("the account was not attached: {error}"))
}

/// Install the answer on `url`, then bring the account down the way a
/// passkey sign-in does once its root is recorded.
async fn finish(state: &TonkState, url: &str) -> Result<(), String> {
    let delivered = answer(state, url).await?;
    adopt(state, delivered).await?;
    super::account::finish_link(state)
        .await
        .map_err(|error| format!("the account did not finish linking: {error}"))?;
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
        match start(&tonk, &command.via.0).await {
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
        let tonk = self.state().read().await;
        super::ceremony::report(&tonk, ceremony::SIGN_IN_VIA, ceremony_state::WORKING, "").await;
        match finish(&tonk, &command.url.0).await {
            Ok(()) => {
                super::ceremony::report(&tonk, ceremony::SIGN_IN_VIA, ceremony_state::DONE, "")
                    .await;
                super::navigate::notify_navigate(self.client(), "/");
            }
            Err(error) => {
                super::ceremony::report(
                    &tonk,
                    ceremony::SIGN_IN_VIA,
                    ceremony_state::FAILED,
                    &error,
                )
                .await
            }
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
        adopt(&state, delivered).await.unwrap();

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

    #[dialog_common::test]
    async fn it_refuses_a_grant_addressed_to_another_device() {
        let state = test_state_without_root().await;
        let elsewhere = Ed25519Signer::import(&[9; 32]).await.unwrap().did();
        let (encoded, _) = delivery_for(73, &elsewhere).await;
        waiting_on(&state, "r1").await;

        let delivered = answer(&state, &callback("r1", &encoded)).await.unwrap();
        let installed = adopt(&state, delivered).await;
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
