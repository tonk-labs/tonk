use crate::error::AccountTransportKind;
use crate::error::TonkUiError;

fn into_api_error<T>(error: T) -> TonkUiError
where
    T: std::fmt::Display,
{
    TonkUiError::ApiError(format!("{error}"))
}

fn account_boundary_error(
    transport_kind: AccountTransportKind,
    status: Option<u16>,
    service_code: Option<String>,
    diagnostic: impl Into<String>,
) -> TonkUiError {
    TonkUiError::AccountApi {
        transport_kind,
        status,
        service_code,
        diagnostic: diagnostic.into(),
    }
}

/// Returns the page origin (`http://host:port`). Used by API
/// helpers to build absolute URLs against the worker's routes.
pub fn origin() -> String {
    web_sys::window()
        .expect("Could not access window")
        .location()
        .origin()
        .expect("Could not read window location")
}

/// The branch this profile is on, as the top page resolved it at boot.
/// The bridge that knows it exists only on the page; off wasm the
/// endpoints are exercised against `main`.
pub(crate) fn profile_branch() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        tonk_host::bridge::profile_branch()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        "main".to_owned()
    }
}

/// Profile-side counterpart to [`evaluate`] — POSTs to
/// Assert a claim on the profile's active branch.
///
/// The page's way of causing an effect: a transient lands, its command
/// runs, and the outcome comes back as facts the page is subscribed to.
/// Nothing is read from the answer beyond whether the commit landed.
pub async fn transact_profile(claim: serde_json::Value) -> Result<(), TonkUiError> {
    tonk_host::ready::wait().await;
    // The branch this profile is on, not `main`: after a sign-out or an
    // added account the profile is on another branch, and a ceremony's
    // commands answer on the branch they were asked on.
    let branch = profile_branch();
    let response = reqwest::Client::new()
        .post(format!("{}/api/profile/branch/{branch}/transact", origin()))
        .json(&claim)
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        Ok(())
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "POST /api/profile/branch/{branch}/transact returned {status}: {text}"
        )))
    }
}

/// Outcome of [`join`] — invite redemption.
///
/// Distinguishes "name already taken" from other failures so the
/// `/join` form can keep the user on the page with a rename
/// prompt rather than a generic error. Note that "you already have
/// this space" is *not* an error here: the worker treats that
/// branch as success ([`JoinResponse::Renewed`]).
#[derive(Debug)]
pub enum JoinError {
    /// The chosen name is taken by an unrelated space. Recipient
    /// should retry with a different name.
    NameTaken,
    /// Any other failure — network, malformed invite, 5xx, etc.
    Other(TonkUiError),
}

impl From<TonkUiError> for JoinError {
    fn from(error: TonkUiError) -> Self {
        Self::Other(error)
    }
}

/// This profile's DID: the profile's own replica row names it.
pub async fn profile_did() -> Result<Option<String>, TonkUiError> {
    let body = serde_json::json!({
        "predicate": { "with": {
            "profile": { "the": "xyz.tonk.replica/profile", "as": "Entity", "cardinality": "one" },
            "kind": { "the": "xyz.tonk.replica/kind", "as": "Entity", "cardinality": "one" }
        } },
        "terms": {
            "this": { "?": { "name": "replica" } },
            "profile": { "?": { "name": "profile" } },
            "kind": tonk_schema::Replica::PROFILE
        }
    });
    let rows = query_profile(&body).await?;
    Ok(first_field(&rows, "profile"))
}

/// The local root this device holds a grant from, as its overlay rows
/// describe it. The grant itself never leaves the worker.
pub struct LocalRoot {
    /// The account root DID.
    pub root: String,
    /// The passkey's WebAuthn credential id.
    pub credential: String,
    /// The recorded encryption key, once one is.
    pub key: Option<String>,
}

/// Read this device's local root rows; `None` when it holds no root.
pub async fn local_root() -> Result<Option<LocalRoot>, TonkUiError> {
    let body = serde_json::json!({
        "predicate": { "with": {
            "root": { "the": "xyz.tonk.local-root/root", "as": "Entity", "cardinality": "one" },
            "credential": { "the": "xyz.tonk.local-root/credential", "as": "Text", "cardinality": "one" }
        } },
        "terms": {
            "this": tonk_schema::LocalRootState::ENTITY,
            "root": { "?": { "name": "root" } },
            "credential": { "?": { "name": "credential" } }
        }
    });
    let rows = query_profile(&body).await?;
    let (Some(root), Some(credential)) =
        (first_field(&rows, "root"), first_field(&rows, "credential"))
    else {
        return Ok(None);
    };
    let body = serde_json::json!({
        "predicate": { "with": {
            "key": { "the": "xyz.tonk.local-root/encryption-key", "as": "Entity", "cardinality": "one" }
        } },
        "terms": {
            "this": tonk_schema::LocalRootState::ENTITY,
            "key": { "?": { "name": "key" } }
        }
    });
    let key = first_field(&query_profile(&body).await?, "key");
    Ok(Some(LocalRoot {
        root,
        credential,
        key,
    }))
}

/// Whether this device holds an account's authority on the active branch:
/// the `state:account-link` row is present.
pub async fn account_linked() -> Result<bool, TonkUiError> {
    let body = serde_json::json!({
        "predicate": { "with": {
            "account": { "the": "xyz.tonk.link/account", "as": "Entity", "cardinality": "one" }
        } },
        "terms": {
            "this": tonk_schema::AccountLink::ENTITY,
            "account": { "?": { "name": "account" } }
        }
    });
    Ok(first_field(&query_profile(&body).await?, "account").is_some())
}

/// The `account/save-encryption-key` claim: record `key` with the local
/// root the worker already holds.
pub(crate) fn save_encryption_key_claim(key: &str) -> serde_json::Value {
    serde_json::json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": "Record the account's encryption key with this device's root.",
                        "with": {
                            "key": { "the": "xyz.tonk.save-encryption-key/key", "as": "Text" }
                        }
                    }
                },
                "parameters": { "key": key }
            }
        }]
    })
}

/// The string value of `field` in the first row that has one.
fn first_field(rows: &serde_json::Value, field: &str) -> Option<String> {
    rows.as_array()?
        .iter()
        .find_map(|row| row["fields"][field].as_str())
        .map(str::to_owned)
}

/// Ask the worker for a sync drain soon.
///
/// The registering ceremony's activation signal is the account sweep's
/// own pull turning from refused to served, so its freshness is the
/// drain cadence. While the ceremony waits it nudges on its own clock
/// instead of the background heartbeat's; the drain coalesces concurrent
/// nudges, so an extra one costs nothing.
pub fn kick_sync() {
    #[cfg(target_arch = "wasm32")]
    tonk_host::keepalive();
}

/// Run a one-shot query against the profile's active branch and return
/// its rows.
///
/// The read half of the page's contract with the worker: commands write
/// facts, and the page reads them back through the same query any view
/// uses, not through an endpoint shaped for one caller.
pub async fn query_profile(body: &serde_json::Value) -> Result<serde_json::Value, TonkUiError> {
    tonk_host::ready::wait().await;
    let branch = profile_branch();
    let response = reqwest::Client::new()
        .post(format!("{}/api/profile/branch/{branch}/query", origin()))
        .json(body)
        .send()
        .await
        .map_err(into_api_error)?;
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(TonkUiError::ApiError(format!(
            "POST /api/profile/branch/{branch}/query returned {status}: {text}"
        )));
    }
    response.json().await.map_err(into_api_error)
}

/// The `account/check-activation` claim: ask the access service whether
/// this account is active, answered by the activation fact.
pub(crate) fn check_activation_claim(at: u64) -> serde_json::Value {
    serde_json::json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": "Ask the access service whether this account is active.",
                        "with": {
                            "at": { "the": "xyz.tonk.check-activation/at", "as": "UnsignedInteger" }
                        }
                    }
                },
                "parameters": { "at": at }
            }
        }]
    })
}

/// Ask the worker to check activation with the access service. The answer
/// lands as the account's activation fact; see [`account_activated`].
pub async fn check_activation() -> Result<(), TonkUiError> {
    transact_profile(check_activation_claim(js_sys::Date::now() as u64)).await
}

/// Whether the account's activation fact is on the profile — presence is
/// the whole answer.
pub async fn account_activated() -> Result<bool, TonkUiError> {
    let body = serde_json::json!({
        "predicate": { "with": {
            "activated_at": {
                "the": "xyz.tonk.account/activated-at", "as": "UnsignedInteger",
                "cardinality": "one"
            }
        } },
        "terms": {
            "this": { "?": { "name": "account" } },
            "activated_at": { "?": { "name": "activated_at" } },
        }
    });
    let rows = query_profile(&body).await?;
    Ok(rows.as_array().is_some_and(|rows| !rows.is_empty()))
}

/// Ask the worker where a first visit to the root should land, and wait for
/// its answer: the Welcome space's path, or `None` to stay put.
///
/// Asked before the page mounts anything, so the wait is on a short beat;
/// setting Welcome up the first time fetches its content, so it is long.
pub async fn open_welcome() -> Result<Option<String>, TonkUiError> {
    let at = js_sys::Date::now() as u64;
    transact_profile(serde_json::json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": "Set up the Welcome space on a first visit.",
                        "with": {
                            "at": { "the": "xyz.tonk.open-welcome/at", "as": "UnsignedInteger" }
                        }
                    }
                },
                "parameters": { "at": at }
            }
        }]
    }))
    .await?;
    let answer = serde_json::json!({
        "predicate": { "with": {
            "at": { "the": "xyz.tonk.welcome/answered-at", "as": "UnsignedInteger", "cardinality": "one" },
            "path": { "the": "xyz.tonk.welcome/path", "as": "Text", "cardinality": "one" }
        } },
        "terms": {
            "this": tonk_schema::WelcomeAnswer::ENTITY,
            "at": { "?": { "name": "at" } },
            "path": { "?": { "name": "path" } }
        }
    });
    for _ in 0..WELCOME_ATTEMPTS {
        let rows = query_profile(&answer).await?;
        if let Some(fields) = rows
            .as_array()
            .and_then(|rows| rows.first())
            .map(|row| &row["fields"])
            && fields["at"].as_u64() == Some(at)
        {
            let path = fields["path"].as_str().unwrap_or_default();
            return Ok((!path.is_empty()).then(|| path.to_owned()));
        }
        sleep(WELCOME_BEAT_MS).await;
    }
    Err(TonkUiError::ApiError(
        "the worker did not say where to land".to_owned(),
    ))
}

/// The beat [`open_welcome`] polls on, and how many beats it waits: short
/// enough not to hold a returning visit's first paint, long enough in
/// total for a first visit to set Welcome up.
const WELCOME_BEAT_MS: i32 = 25;
const WELCOME_ATTEMPTS: usize = 2400;

/// Resolve after `millis`.
async fn sleep(millis: i32) {
    let sleep = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, millis);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(sleep).await;
}

/// Poll until this device holds a recovered credential, or give up.
///
/// Custody is the one post-passkey step that can genuinely fail, so it
/// is the only one whose failure the ceremony reports. A local root row is
/// the answer; anything else is not yet.
pub async fn await_custody() -> bool {
    poll_until(RECOVERY_ATTEMPTS, || async {
        matches!(local_root().await, Ok(Some(_)))
    })
    .await
}

/// Wait until the local root records `key`, or give up.
pub async fn await_encryption_key(key: &str) -> bool {
    poll_until(RECOVERY_ATTEMPTS, || async {
        matches!(local_root().await, Ok(Some(root)) if root.key.as_deref() == Some(key))
    })
    .await
}

/// How long custody recovery waits before the ceremony stops narrating
/// it. Generous, because the point is to describe a slow network rather
/// than to time it out — but bounded, because a phase that never answers
/// must not strand the screen.
const RECOVERY_ATTEMPTS: usize = 120;

/// The beat between polls. Long enough not to hammer the worker, short
/// enough that a phase which resolves quickly reads as immediate.
const POLL_EVERY_MS: i32 = 250;

/// Run `check` until it answers true or `attempts` are spent.
async fn poll_until<F, Fut>(attempts: usize, check: F) -> bool
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..attempts {
        if check().await {
            return true;
        }
        sleep(POLL_EVERY_MS).await;
    }
    false
}

/// Save the account name and wait for its durable write before reporting
/// success: assert the `profile/rename` command, then wait for the
/// account's display-name fact to say `name`.
pub async fn set_display_name(name: &str) -> Result<String, TonkUiError> {
    let root = local_root().await?.map(|root| root.root).ok_or_else(|| {
        account_boundary_error(
            AccountTransportKind::Http,
            None,
            None,
            "there is no account on this device to name",
        )
    })?;
    transact_profile(profile_rename_claim(name)).await?;
    let saved = poll_until(RECOVERY_ATTEMPTS, || async {
        account_display_name(&root).await.ok().flatten().as_deref() == Some(name)
    })
    .await;
    if saved {
        Ok(name.to_owned())
    } else {
        Err(account_boundary_error(
            AccountTransportKind::Http,
            None,
            None,
            "the account display name was not recorded",
        ))
    }
}

/// The account's chosen display name, read off the profile branch.
pub async fn account_display_name(root: &str) -> Result<Option<String>, TonkUiError> {
    let body = serde_json::json!({
        "predicate": { "with": {
            "name": { "the": "xyz.tonk.account/display-name", "as": "Text", "cardinality": "one" }
        } },
        "terms": {
            "this": root,
            "name": { "?": { "name": "name" } }
        }
    });
    Ok(first_field(&query_profile(&body).await?, "name"))
}

/// The `profile/rename` claim, in the shape its command decodes.
fn profile_rename_claim(name: &str) -> serde_json::Value {
    serde_json::json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": "Rename the signed-in member.",
                        "with": {
                            "name": { "the": "xyz.tonk.command.profile-rename/name", "as": "Text" }
                        }
                    }
                },
                "parameters": { "name": name }
            }
        }]
    })
}
