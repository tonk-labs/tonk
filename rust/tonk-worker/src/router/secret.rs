//! Sealed command fields, on the worker's side: the key that opens them,
//! where invokers find its public half, and what a provider calls to
//! read one.
//!
//! The key is derived from the operator ([`tonk_identity::secret`]), so
//! this worker and its successors open the same blobs until the profile or
//! account changes. Its public half is published on the overlay of the
//! profile's meta branch ([`publish`]), where the host and the CLI read it
//! before sealing a field declared `as: secret`.
//!
//! The dispatcher refuses a command whose sealed field does not open
//! ([`super::command::dispatch_committed`]), so a provider that reveals a
//! field it was handed only fails if the worker's key could not be
//! derived.

use tonk_common::log;
use tonk_schema::invocation::Failure;
use tonk_schema::secret::{CommandSecretRecipient, SealedAttribute, SecretError, SecretKey};

use super::CommandEnv;
use crate::worker::TonkState;

/// The key this worker opens sealed fields with, derived from its
/// operator. `None` when the operator's key cannot derive one (it is not
/// an Ed25519 key, or a platform crypto call failed); every sealed field
/// is then refused.
pub async fn key(tonk: &TonkState) -> Option<SecretKey> {
    match tonk_schema::secret::derive_key(tonk.operator.credential().signer()).await {
        Ok(key) => Some(key),
        Err(error) => {
            log!("command secrets: no key derived; sealed fields are refused: {error}");
            None
        }
    }
}

/// Publish the recipient invokers seal to, on the overlay of the
/// profile's meta branch. Called whenever a state is built, so a profile
/// or account switch replaces it with the new operator's.
pub async fn publish(tonk: &TonkState) {
    let Some(key) = key(tonk).await else {
        return;
    };
    if let Err(error) = tonk
        .reactor
        .profile_repository()
        .branch(super::repository::META_BRANCH)
        .overlay()
        .assert(CommandSecretRecipient::new(&key.recipient()))
        .write()
        .perform(&tonk.operator)
        .await
    {
        log!("command secrets: the recipient was not published: {error}");
    }
}

/// The failure a command reports when a sealed field does not open.
pub fn refusal(error: &SecretError) -> Failure {
    Failure::new(error.to_string())
}

impl CommandEnv {
    /// Reveal a sealed field of the command this env runs.
    ///
    /// The dispatcher has already refused a command whose sealed fields
    /// do not open, so this fails only when the key cannot be derived.
    pub async fn reveal<A: SealedAttribute>(&self, field: &A) -> Result<A::Plain, Failure> {
        let tonk = self.state().read().await;
        let key = key(&tonk)
            .await
            .ok_or_else(|| refusal(&SecretError::EarlierSession))?;
        tonk_schema::secret::reveal(&key, field).map_err(|error| refusal(&error))
    }
}

#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
mod tests {
    //! Native only: the dispatch these drive runs the real `tonk/join`
    //! provider over native storage. On wasm the same dispatcher and
    //! providers run; `tonk_identity::secret`'s tests pin the sealing
    //! itself on both targets.

    use dialog_artifacts::{Changes, Entity, Statement as _, Value};
    use dialog_query::{Output as _, Query, Term};
    use dialog_varsig::Principal as _;
    use tonk_identity::sealed::RecipientKey;
    use tonk_schema::invocation::{Case, Output, Status, case};
    use tonk_schema::secret::CommandSecretRecipient;

    use crate::router::command::tests::native::test_state;
    use crate::router::{AppState, CommandOrigin, dispatch};

    const SEALED_URL: &str = "xyz.tonk.command.join/sealed-url";
    const PLAIN_URL: &str = "xyz.tonk.command.join/url";
    /// An invite whose delegation cannot be read: the join provider fails
    /// it as malformed without any network, which is how these tell that
    /// it ran.
    const INVITE: &str = "https://tonk.test/join?access=not-a-delegation#secret-seed";
    const SEED: &[u8] = b"secret-seed";

    /// The recipient as an invoker finds it: off the overlay of the
    /// profile's meta branch.
    async fn published(state: &AppState) -> RecipientKey {
        let tonk = state.read().await;
        let rows: Vec<CommandSecretRecipient> = tonk
            .reactor
            .profile_repository()
            .branch(crate::router::repository::META_BRANCH)
            .acquire(&tonk.operator)
            .await
            .unwrap()
            .handle()
            .query()
            .select(Query::<CommandSecretRecipient> {
                this: Term::var("this"),
                recipient: Term::var("recipient"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap();
        let [row] = rows.as_slice() else {
            panic!("expected one published recipient, got {rows:?}");
        };
        RecipientKey::try_from(&row.recipient.0.parse().unwrap()).unwrap()
    }

    async fn published_state() -> AppState {
        let state = test_state().await;
        super::publish(&*state.read().await).await;
        state
    }

    /// Every branch overlay a command could have written to: the active
    /// profile branch (invocations) and meta (the join's status).
    async fn overlays(state: &AppState) -> Vec<Changes> {
        let tonk = state.read().await;
        let mut overlays = Vec::new();
        for branch in [
            tonk.active_branch.clone(),
            crate::router::repository::META_BRANCH.to_owned(),
        ] {
            overlays.push(
                tonk.reactor
                    .profile_repository()
                    .branch(&branch)
                    .acquire(&tonk.operator)
                    .await
                    .unwrap()
                    .handle()
                    .overlay()
                    .export(),
            );
        }
        overlays
    }

    /// Whether any overlay holds `needle`, in text or in bytes.
    async fn holds(state: &AppState, needle: &[u8]) -> bool {
        overlays(state).await.iter().any(|overlay| {
            overlay.iter().any(|(_, _, change)| match change.value() {
                Value::String(text) => text
                    .as_bytes()
                    .windows(needle.len())
                    .any(|window| window == needle),
                Value::Bytes(bytes) => bytes.windows(needle.len()).any(|window| window == needle),
                _ => false,
            })
        })
    }

    /// What the overlays hold for `of`'s `the`.
    async fn held(state: &AppState, of: &str, the: impl ToString) -> Vec<Value> {
        let of: Entity = of.parse().unwrap();
        let the = the.to_string();
        overlays(state)
            .await
            .iter()
            .flat_map(|overlay| {
                overlay
                    .iter()
                    .filter(|(entity, relation, change)| {
                        **entity == of
                            && relation.to_string() == the
                            && matches!(change, dialog_artifacts::Change::Assert(..))
                    })
                    .map(|(_, _, change)| change.value().clone())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Whether the join provider ran: it records an attempt's status.
    async fn join_ran(state: &AppState) -> bool {
        !held(state, "tonk:join/status", "xyz.tonk.join/status")
            .await
            .is_empty()
    }

    /// A `tonk/join` transient on `of` carrying `value` at `attribute`,
    /// stamped as the palette stamps what it runs, so the dispatcher
    /// keeps an invocation of it.
    fn join(of: &str, attribute: &str, value: Value) -> Changes {
        use tonk_schema::invocation::command::{Concept, InvokedAt};
        let of: Entity = of.parse().unwrap();
        let mut changes = Changes::new();
        crate::router::claim::RawClaim {
            the: attribute.parse().unwrap(),
            of: of.clone(),
            is: value,
            policy: dialog_artifacts::Pick::Last,
        }
        .assert(&mut changes);
        InvokedAt::of(of.clone())
            .is(InvokedAt(1.0))
            .assert(&mut changes);
        Concept::of(of)
            .is(Concept("tonk:join".parse().unwrap()))
            .assert(&mut changes);
        changes
    }

    /// The message a failed invocation of `of` recorded.
    async fn failure(state: &AppState, of: &str) -> String {
        assert_eq!(
            held(state, of, Case::the()).await,
            vec![Value::Entity(case::FAILED.parse().unwrap())],
            "{of} failed"
        );
        let error = crate::router::invocation::error_entity(&of.parse().unwrap());
        assert_eq!(
            held(state, of, Output::the()).await,
            vec![Value::Entity(error.clone())]
        );
        match held(state, &error.to_string(), Status::the())
            .await
            .as_slice()
        {
            [Value::String(message)] => message.clone(),
            other => panic!("expected one message, got {other:?}"),
        }
    }

    /// The worker publishes where to seal to, derived from its operator:
    /// a reboot over the same profile publishes the same recipient, so a
    /// blob sealed before a worker restart still opens after it.
    #[dialog_common::test]
    async fn the_recipient_is_published_at_boot_and_survives_a_reboot() {
        let name = format!("command-secret-test-{}", rand::random::<u64>());
        let directory = dialog_effects::storage::Directory::At(
            std::env::temp_dir()
                .join(&name)
                .to_string_lossy()
                .into_owned(),
        );
        let boot = || async {
            let (storage, profile) = crate::device::open_profile_at(&name, directory.clone())
                .await
                .unwrap();
            let registry = crate::device::Registry::new(name.clone(), directory.clone());
            crate::worker::boot_state(storage, name.clone(), profile, registry)
                .await
                .unwrap()
        };
        let first = boot().await;
        let first = std::sync::Arc::new(crate::StateLock::new(first));
        let before = published(&first).await;
        let field = before.seal_secret(INVITE.as_bytes(), SEALED_URL).unwrap();
        drop(first);

        let again = boot().await;
        let again = std::sync::Arc::new(crate::StateLock::new(again));
        assert_eq!(published(&again).await, before);
        let key = super::key(&*again.read().await).await.unwrap();
        assert_eq!(key.recipient().did(), before.did());
        assert_eq!(
            key.reveal(&field, SEALED_URL).unwrap().as_slice(),
            INVITE.as_bytes(),
            "what was sealed before the reboot opens after it"
        );
    }

    /// A join sealed to the published recipient runs, and nothing it
    /// leaves in any overlay holds the seed: the invocation copies the
    /// sealed bytes, not the url.
    #[dialog_common::test]
    async fn a_sealed_secret_runs_and_no_plaintext_reaches_the_overlay() {
        let state = published_state().await;
        let sealed = published(&state)
            .await
            .seal_secret(INVITE.as_bytes(), SEALED_URL)
            .unwrap();
        dispatch(
            &state,
            CommandOrigin::default(),
            join("id:sealed-join", SEALED_URL, Value::Bytes(sealed.clone())),
        )
        .await;

        assert!(
            join_ran(&state).await,
            "the provider revealed the url and ran"
        );
        assert_eq!(
            failure(&state, "id:sealed-join").await,
            tonk_worker_api::JoinFailureKind::Malformed.message(),
            "it failed on the invite, not on the seal"
        );
        assert_eq!(
            held(&state, "id:sealed-join", SEALED_URL).await,
            vec![Value::Bytes(sealed)],
            "the invocation holds the field as sealed"
        );
        assert!(!holds(&state, SEED).await, "no overlay holds the seed");
    }

    /// Plain text where a secret belongs fails the command with its own
    /// error, before any provider runs, and the invocation keeps none of
    /// it.
    #[dialog_common::test]
    async fn a_plaintext_secret_fails_closed() {
        let state = published_state().await;
        dispatch(
            &state,
            CommandOrigin::default(),
            join(
                "id:plain-join",
                SEALED_URL,
                Value::String(INVITE.to_owned()),
            ),
        )
        .await;

        assert_eq!(
            failure(&state, "id:plain-join").await,
            tonk_schema::secret::SecretError::NotSealed.to_string()
        );
        assert!(!join_ran(&state).await, "the command never ran");
        assert!(held(&state, "id:plain-join", SEALED_URL).await.is_empty());
        assert!(!holds(&state, SEED).await, "no overlay holds the seed");
    }

    /// A blob sealed to another worker key, as one from before a profile
    /// switch is, is refused as sealed for an earlier session and never
    /// runs.
    #[dialog_common::test]
    async fn a_blob_sealed_for_another_session_is_refused() {
        let state = published_state().await;
        let elsewhere = dialog_credentials::Ed25519Signer::import(&[9u8; 32])
            .await
            .unwrap();
        let elsewhere = tonk_schema::secret::SecretKey::derive(&elsewhere)
            .await
            .unwrap();
        assert_ne!(elsewhere.recipient(), published(&state).await);
        let sealed = elsewhere
            .recipient()
            .seal_secret(INVITE.as_bytes(), SEALED_URL)
            .unwrap();
        dispatch(
            &state,
            CommandOrigin::default(),
            join("id:stale-join", SEALED_URL, Value::Bytes(sealed)),
        )
        .await;

        let message = failure(&state, "id:stale-join").await;
        assert_eq!(
            message,
            tonk_schema::secret::SecretError::EarlierSession.to_string()
        );
        assert_eq!(message, "the secret was sealed for an earlier session");
        assert!(!join_ran(&state).await, "the command never ran");
    }

    /// A page seeded before secrets were sealed still sends the url in
    /// plain text on the old attribute. It still joins, and its
    /// invocation copies no url.
    #[dialog_common::test]
    async fn a_legacy_plaintext_join_runs_without_copying_its_url() {
        let state = published_state().await;
        dispatch(
            &state,
            CommandOrigin::default(),
            join(
                "id:legacy-join",
                PLAIN_URL,
                Value::String(INVITE.to_owned()),
            ),
        )
        .await;

        assert!(join_ran(&state).await, "the legacy shape still runs");
        assert_eq!(
            failure(&state, "id:legacy-join").await,
            tonk_worker_api::JoinFailureKind::Malformed.message()
        );
        assert!(held(&state, "id:legacy-join", PLAIN_URL).await.is_empty());
        assert!(!holds(&state, SEED).await, "no overlay holds the seed");
    }
}
