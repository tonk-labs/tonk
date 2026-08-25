//! Getting the account's encryption key onto a device that needs it.
//!
//! Custody seals to the account's X25519 recipient, which only a holder
//! of the account secret can derive. The onboarding account's secret is
//! local, so its recipient is derived on demand. A passkey account's
//! secret only exists inside a WebAuthn assertion on a page, so a linked
//! device whose root record predates the key asks the page that
//! originated the operation to run one (`request_webauthn`), and waits
//! for the page to save the key with the root (`POST /api/identity/root`)
//! before continuing. Nothing is replayed: the operation that needed the
//! key simply resumes once it is there.

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use crate::TonkWorkerError;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use crate::router::{AppState, ClientId};
use dialog_varsig::Did;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
thread_local! {
    /// Operations waiting for the page to record an encryption key.
    static WAITERS: std::cell::RefCell<Vec<tokio::sync::oneshot::Sender<Did>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// How long an operation waits for the page's assertion before giving
/// up. Generous: the user has to touch a passkey.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const ASSERTION_TIMEOUT: web_time::Duration = web_time::Duration::from_secs(120);

/// Wake every operation waiting for the key. Called by the root save
/// whenever a record carrying a recipient lands.
pub(crate) fn notify_encryption_key(recipient: &Did) {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    WAITERS.with(|waiters| {
        for waiter in waiters.borrow_mut().drain(..) {
            let _ = waiter.send(recipient.clone());
        }
    });
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    let _ = recipient;
}

/// Make sure a custody recipient is obtainable before an operation that
/// custodies a seed takes the state lock.
///
/// Returns at once when the device has no passkey root (the onboarding
/// recipient derives locally) or when the root record already carries the
/// key. Otherwise asks `client` for a passkey assertion and waits for the
/// key to be saved. Must be called WITHOUT the state lock held: the page
/// answers through `/api/identity/root`, which needs it.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn ensure_recipient(
    state: &AppState,
    client: Option<&ClientId>,
) -> Result<(), TonkWorkerError> {
    let root = {
        let tonk = state.read().await;
        match super::identity::local_root(&tonk).await {
            Ok(root) => root,
            Err(TonkWorkerError::RootRequired) => return Ok(()),
            Err(error) => return Err(error),
        }
    };
    if root.encryption_key.is_some() {
        return Ok(());
    }
    {
        let tonk = state.read().await;
        if super::account_state::published_encryption_key(&tonk, &root.root_did)
            .await?
            .is_some()
        {
            return Ok(());
        }
    }
    let Some(client) = client else {
        return Err(TonkWorkerError::Conflict(
            "the account has not published its encryption key on this device, and no page \
             asked for this operation, so no passkey assertion can derive it"
                .to_string(),
        ));
    };
    request_and_wait(client).await
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn request_and_wait(client: &ClientId) -> Result<(), TonkWorkerError> {
    // Registered before the request goes out, so an answer that arrives
    // faster than this task resumes is not missed.
    let receiver = wait_for_key();
    super::navigate::request_webauthn(client, tonk_worker_api::ENCRYPTION_KEY_REQUEST).await?;
    await_key(receiver, ASSERTION_TIMEOUT).await.map(|_| ())
}

/// Register for the next recorded encryption key.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn wait_for_key() -> tokio::sync::oneshot::Receiver<Did> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    WAITERS.with(|waiters| waiters.borrow_mut().push(sender));
    receiver
}

/// Wait for a registered key, giving up after `timeout`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn await_key(
    receiver: tokio::sync::oneshot::Receiver<Did>,
    timeout: web_time::Duration,
) -> Result<Did, TonkWorkerError> {
    use futures_util::future::{Either, select};

    let timeout = Box::pin(crate::r#async::sleep(timeout));
    match select(Box::pin(receiver), timeout).await {
        Either::Left((Ok(recipient), _)) => Ok(recipient),
        Either::Left((Err(_), _)) => Err(TonkWorkerError::Internal(
            "the encryption-key waiter was dropped".to_string(),
        )),
        Either::Right(_) => Err(TonkWorkerError::Conflict(
            "no passkey assertion answered in time; the account's encryption key is still \
             unpublished on this device"
                .to_string(),
        )),
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use crate::router::tests::{test_state, test_state_without_account, test_state_without_root};
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    /// An onboarding device derives its recipient locally: nothing to ask.
    #[dialog_common::test]
    async fn it_needs_no_assertion_without_a_passkey_root() {
        let state = Arc::new(RwLock::new(test_state_without_root().await));
        ensure_recipient(&state, None).await.unwrap();
    }

    /// The fixture's root carries the key a ceremony would have recorded.
    #[dialog_common::test]
    async fn it_needs_no_assertion_when_the_root_carries_the_key() {
        let state = Arc::new(RwLock::new(test_state().await));
        ensure_recipient(&state, None).await.unwrap();
    }

    /// The page answers by saving the key with the root; that save is
    /// what wakes the waiting operation.
    #[dialog_common::test]
    async fn it_wakes_the_waiter_when_the_root_save_records_the_key() {
        let state = test_state_without_account().await;
        let receiver = wait_for_key();
        let recipient =
            tonk_identity::envelope::AccountSecret::from_bytes(zeroize::Zeroizing::new([9u8; 32]))
                .encryption_key()
                .recipient()
                .did();
        let root = super::super::identity::local_root(&state).await.unwrap();
        super::super::identity::persist_root(
            &state,
            tonk_worker_api::SaveRootRequest {
                credential_id: root.credential_id,
                delegation_hex: hex::encode(root.bytes),
                passkey: None,
                encryption_key: Some(recipient.to_string()),
            },
        )
        .await
        .unwrap();
        let woken = await_key(receiver, web_time::Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(woken, recipient);
    }

    /// Without a page to ask, a linked device lacking the key refuses
    /// rather than waiting on nothing.
    #[dialog_common::test]
    async fn it_refuses_without_a_client_to_ask() {
        let state = Arc::new(RwLock::new(test_state_without_account().await));
        strip_recorded_key(&state).await;
        let error = ensure_recipient(&state, None).await.unwrap_err();
        assert!(matches!(error, TonkWorkerError::Conflict(_)), "{error}");
    }

    /// Overwrite the local root record without its recipient and drop the
    /// published fact, the shape of a device linked before the key
    /// existed.
    async fn strip_recorded_key(state: &AppState) {
        use tonk_schema::prelude::DidExt as _;
        let tonk = state.read().await;
        let root = super::super::identity::local_root(&tonk).await.unwrap();
        super::super::identity::forget_encryption_key(&tonk)
            .await
            .unwrap();
        let published =
            super::super::account_state::published_encryption_key(&tonk, &root.root_did)
                .await
                .unwrap()
                .expect("the fixture published one");
        tonk.reactor
            .profile_repository()
            .branch(tonk_account::MAIN_BRANCH)
            .transaction()
            .retract(tonk_schema::AccountEncryptionKey::new(
                root.root_did.this(),
                published.this(),
            ))
            .commit()
            .perform(&tonk.operator)
            .await
            .unwrap();
    }
}
