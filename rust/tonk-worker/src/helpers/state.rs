//! A real `TonkState` for tests — the worker's own storage, profile,
//! root grant and attached account, with no fakes anywhere in it.
//!
//! Lives here rather than in `router.rs`'s test module because this is
//! not the only crate that needs a real worker to test against.
//! `tonk-display` boots one of these in a browser and drives the whole
//! stack through it — the real host dispatching consumer events, the
//! real router answering them, the real query engine underneath — which
//! is the only way the wire shapes between those layers get checked
//! against anything but each other.
//!
//! Wasm-only: `TonkState` needs IndexedDB. That works in a window as
//! well as in a worker, so a DOM test can use it.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use dialog_credentials::Ed25519Signer;
use dialog_storage::provider::storage::Storage;
use dialog_varsig::Principal as _;
use tonk_schema::prelude::DidExt as _;

use crate::worker::{DefaultSpace, TonkState};

/// A random id minted once per test *process*, mixed into every profile
/// name so two runs never collide on storage a shared browser profile
/// kept between them.
fn session_nonce() -> u32 {
    use std::sync::OnceLock;
    static NONCE: OnceLock<u32> = OnceLock::new();
    *NONCE.get_or_init(rand::random::<u32>)
}

/// Creates a test state with the default storage backend.
///
/// The state has a profile and operator but *no* repository —
/// tests that need one call [`put_repo`] with a display label and
/// use the minted routing key it returns. Every create mints a
/// fresh identity for the repos it makes, but the profile itself
/// is durable IndexedDB state keyed by name: each call mints its
/// own unique profile name so tests that rename or restamp the
/// profile never bleed into one another.
///
/// The sequence number alone is unique only *within* a run —
/// `test-tonk-3` is whichever test happened to run third — so a
/// runner that reuses a browser profile (safaridriver, a persistent
/// Chrome user-data-dir) would hand run N's leftover IndexedDB to
/// run N+1's third test, reviving the order dependence in cross-run
/// form. `wasm-bindgen-test-runner`'s throwaway Chrome profile hides
/// that today; the [`session_nonce`] makes it unconditional.
pub async fn test_state_without_root() -> TonkState {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let profile_name = format!(
        "test-tonk-{}-{}",
        session_nonce(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    );

    crate::patch_idb_versionchange();
    let storage = Storage::<DefaultSpace>::default();
    let profile = dialog_peer::OpenPeer::open(dialog_effects::storage::Location::new(
        dialog_effects::storage::Directory::Profile,
        &profile_name,
    ))
    .perform(&storage)
    .await
    .expect("Failed to create test profile");

    let session = crate::session::open(&profile)
        .await
        .expect("Failed to open a test signing session");

    let reactor = crate::Reactor::new(profile.credential().clone());
    // The registry mirrors production shape — the state's own profile
    // is the registry profile, exactly as `Registry::device()` signs
    // as `tonk` until the first rotation. Uniquely named per state,
    // so tests neither collide with each other nor touch the real
    // registry, while rotated/activated profiles still resolve in the
    // same directory the test profile itself lives in.
    let registry = crate::device::Registry {
        profile: profile_name.clone(),
        directory: dialog_effects::storage::Directory::Profile,
    };
    TonkState {
        seed_upgrades: Default::default(),
        profile,
        operator: session.operator,
        storage,
        session_expires_at: session.expires_at,
        profile_name,
        active_branch: crate::router::repository::PROFILE_BRANCH.to_owned(),
        reactor,
        admission: Default::default(),
        reject_admission_content_reads: Default::default(),
        retiring: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        view_bindings: Default::default(),
        bridges: Default::default(),
        sync_queue: Default::default(),
        commands: crate::router::command_providers(),
        clients: Default::default(),
        account_keys: Default::default(),
        profile_library: Default::default(),
        registry,
        profile_transition: Default::default(),
        context_generation: Default::default(),
    }
}

/// The root seed for a test profile, derived from its name.
///
/// Per-profile rather than one shared constant, because the account
/// repository's routing key IS the root's — so every profile sharing a
/// root shares one account repository, and its storage is not scoped by
/// profile the way a space's is. Two tests that link descriptors naming
/// different remotes then fight over the same mount, and the second one
/// to run reads the first one's remote and refuses as a conflict. That
/// is invisible until the ordering shifts, which is exactly the failure
/// [`session_nonce`] exists to prevent one layer down.
///
/// A fold rather than a hash: no dependency, deterministic, and it mixes
/// every byte of the name — which is all that separating test profiles
/// requires.
pub(crate) fn test_root_seed(profile_name: &str) -> [u8; 32] {
    let mut seed = [42u8; 32];
    for (index, byte) in profile_name.as_bytes().iter().enumerate() {
        seed[index % 32] ^= byte.rotate_left((index % 8) as u32);
    }
    seed
}

/// Create an isolated test state with a stable local root grant and no
/// account attached to it.
///
/// The shape a device is in between provisioning a root and finishing
/// sign-up. Only the tests that assert a durable operation refuses want
/// it; everything else wants [`test_state`], because production never
/// creates a root without an account around it.
pub async fn test_state_without_account() -> TonkState {
    let state = test_state_without_root().await;
    persist_test_root(&state).await;
    state
}

/// Persist the test root on `state`, the way a creation or unlock
/// ceremony does: the `root -> device` grant, the recipient custodied
/// seeds are sealed to, and that recipient published on profile main.
/// Returns the root DID.
pub(crate) async fn persist_test_root(state: &TonkState) -> dialog_varsig::Did {
    let root = Ed25519Signer::import(&test_root_seed(&state.profile_name))
        .await
        .unwrap();
    let root_did = root.did();
    let grant = tonk_identity::delegation::mint_device_delegation(root, &state.profile.did())
        .await
        .unwrap();
    // What a creation or unlock ceremony hands back with the root, and
    // what the account sweep then publishes: the recipient custodied
    // seeds are sealed to. Published here directly, since the fixture
    // has no account branch to sweep.
    let recipient = tonk_identity::envelope::AccountSecret::from_bytes(zeroize::Zeroizing::new(
        test_root_seed(&state.profile_name),
    ))
    .secret()
    .did();
    crate::router::identity::persist_root(
        state,
        tonk_worker_api::SaveRootRequest {
            credential_id: "test-credential".to_string(),
            delegation_hex: hex::encode(grant.to_bytes().unwrap()),
            passkey: None,
            encryption_key: Some(recipient.to_string()),
        },
    )
    .await
    .unwrap();
    state
        .reactor
        .profile_repository()
        .branch(&state.active_branch)
        .transaction()
        .assert(tonk_schema::AccountSealedInbox::new(
            root_did.this(),
            recipient.this(),
        ))
        .commit()
        .perform(&state.operator)
        .await
        .expect("the fixture publishes the account's encryption key");
    root_did
}

/// Create an isolated test state with a stable local root grant and an
/// account attached to it — a signed-in device.
pub async fn test_state() -> TonkState {
    let state = test_state_without_account().await;
    crate::router::account::attach_test_account(&state)
        .await
        .unwrap();
    // What a link records on `meta`: the branch the profile is on follows
    // the account's branch on the peer serving it. Without it a sign-out
    // finds the branch following nothing and stays put.
    crate::router::profile::ensure_profile_meta_branch(&state).await;
    let root = crate::router::identity::local_root(&state)
        .await
        .expect("the fixture has a local root")
        .root_did;
    let address = dialog_repository::SiteAddress::from(dialog_remote_ucan::UcanAddress::new(
        crate::router::account::TEST_ACCOUNT_REMOTE,
    ));
    crate::router::account_state::record_account_branch(&state, &root, &address).await;
    state
}
