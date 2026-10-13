//! What a branch runs: the release its head pins.
//!
//! A branch's head carries a boot statement (`dialog_capability`'s
//! [`Boot`]): a line of releases and the one release pinned from it, signed
//! by whoever set it. The statement sits beside the fields the head signs,
//! so it travels wherever the head does (a commit carries it forward, a
//! push publishes it, a pull adopts it) and is read without the branch's
//! tree.
//!
//! This worker does two things with it. It sets one, when the branch is
//! told to pin a release ([`pin`]). And it writes the pin it finds on a head
//! where its origin's shell reads it ([`mirror`]), so that the next load
//! registers the bootloader for that release. Nothing here loads a release:
//! that is the shell's and the bootloader's.

use dialog_capability::identity::Boot;
use dialog_effects::authority::Attest;
use dialog_repository::Revision;
use tonk_common::log;
use tonk_schema::Release;

use crate::{TonkState, TonkWorkerError};

/// The version of the host a release this worker pins needs.
const HOST: u64 = 1;

/// Pin the release `branch` of `repo` runs: sign a statement of it, set it
/// on the branch's head, and record it in the branch.
///
/// The record is what makes the head move: a head that says something new
/// about the same tree is not something a push sends, and a commit on a
/// head carries its statement forward. So the pin reaches every other
/// replica with the next sync, as one head.
pub(crate) async fn pin(
    tonk: &TonkState,
    repo: &str,
    branch: &str,
    src: &str,
    release: &str,
) -> Result<Boot, TonkWorkerError> {
    let reference = if tonk.reactor.profile_key() == repo {
        tonk.reactor.profile_repository().branch(branch)
    } else {
        tonk.reactor.repository(repo).branch(branch)
    };
    let session = reference
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::NotFound(format!("{branch}@{repo}: {error}")))?;
    let handle = session.handle();
    let mut head = handle.revision().ok_or_else(|| {
        TonkWorkerError::Router(format!("{branch}@{repo} has no head to pin a release on"))
    })?;

    let mut boot = Boot {
        v: HOST,
        src: src.to_owned(),
        pin: Some(release.to_owned()),
        seq: head.boot.as_ref().map_or(1, |held| held.seq + 1),
        issuer: tonk.operator.did(),
        signature: Vec::new(),
    };
    boot.signature = Attest::new(boot.payload(&head.branch))
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("could not sign the pin: {error}")))?;
    head.boot = Some(Box::new(boot.clone()));
    handle
        .reset(head)
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("could not set the pin: {error}")))?;
    handle
        .transaction()
        .assert(Release::new(src, release))
        .commit()
        .publish()
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("could not record the pin: {error}")))?;
    tonk.sync_queue.mark_dirty(repo, super::sync::now_millis());
    heed(tonk, repo, handle.revision().as_ref());
    Ok(boot)
}

/// Take up what the head of a branch of `repo` says this origin is to run,
/// where `repo` is what this origin is for: the person's profile on the
/// profile's origin, the space on a space's. Called wherever a head lands.
pub(crate) fn heed(tonk: &TonkState, repo: &str, head: Option<&Revision>) {
    let profile = tonk.reactor.profile_key() == repo;
    if profile != crate::device::spaces_have_origins() {
        return;
    }
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    if let Some(head) = head.cloned() {
        wasm_bindgen_futures::spawn_local(async move { mirror(&head).await });
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    let _ = head;
}

/// The statement `head` carries, where it is its issuer's about this
/// head's branch. One that is not is no statement.
///
/// This says the statement was made by the key it names. It does not say
/// that key may decide what the branch runs: that check is not built.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn stated(head: &Revision) -> Option<&Boot> {
    let boot = head.boot.as_deref()?;
    match boot.verify(&head.branch) {
        Ok(()) => Some(boot),
        Err(error) => {
            log!("boot: ignoring a statement on {}: {error}", head.branch);
            None
        }
    }
}

/// What the shell reads to learn the release its origin pins: the statement
/// of a head, as JSON. `None` for a head that states nothing.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn record(head: &Revision) -> Option<serde_json::Value> {
    let boot = stated(head)?;
    Some(serde_json::json!({
        "v": boot.v,
        "src": boot.src,
        "pin": boot.pin,
        "seq": boot.seq,
        "issuer": boot.issuer.to_string(),
    }))
}

/// Write the pin `head` states where this origin's shell reads it, unless
/// what is written there already is as late. A head that states nothing
/// changes nothing: a writer from before the statement mints heads without
/// one, and that is not a branch unpinning itself.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn mirror(head: &Revision) {
    use wasm_bindgen::{JsCast as _, JsValue};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Cache, CacheStorage, Response};

    const CACHE: &str = "tonk-boot";
    const KEY: &str = "/__boot";

    let Some(record) = record(head) else {
        return;
    };
    let result: Result<bool, JsValue> = async {
        let caches: CacheStorage =
            js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("caches"))?.dyn_into()?;
        let cache: Cache = JsFuture::from(caches.open(CACHE)).await?.dyn_into()?;
        let held = JsFuture::from(cache.match_with_str(KEY)).await?;
        if let Ok(held) = held.dyn_into::<Response>() {
            let written: serde_json::Value =
                serde_wasm_bindgen::from_value(JsFuture::from(held.json()?).await?)
                    .unwrap_or_default();
            if written["seq"].as_u64().unwrap_or(0) >= record["seq"].as_u64().unwrap_or(0) {
                return Ok(false);
            }
        }
        let response = Response::new_with_opt_str(Some(&record.to_string()))?;
        JsFuture::from(cache.put_with_str(KEY, &response)).await?;
        Ok(true)
    }
    .await;
    match result {
        Ok(true) => log!("boot: this origin now pins {}", record["pin"]),
        Ok(false) => {}
        Err(error) => log!("boot: could not write down the pin: {error:?}"),
    }
}

/// Run [`PinRelease`]: pin the release for the branch the command fired on.
///
/// [`PinRelease`]: tonk_schema::command::PinRelease
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::PinRelease> for crate::router::CommandEnv {
    async fn execute(&self, command: tonk_schema::command::PinRelease) {
        let origin = self.origin();
        let tonk = self.state().read().await;
        match pin(
            &tonk,
            &origin.repo,
            &origin.branch,
            &command.src.0,
            &command.pin.0,
        )
        .await
        {
            Ok(boot) => log!(
                "pin release: {}@{} pins {} (statement {})",
                origin.branch,
                origin.repo,
                command.pin.0,
                boot.seq
            ),
            Err(error) => log!(
                "pin release on {}@{} failed: {error}",
                origin.branch,
                origin.repo
            ),
        }
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    use wasm_bindgen::{JsCast as _, JsValue};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Cache, CacheStorage, Response};

    use super::*;
    use crate::router::api_router_with_state;
    use crate::router::tests::{put_repo, test_state};

    const SRC: &str = "https://tonk.test/releases/stable/worker.js";
    const ONE: &str = "https://tonk.test/releases/bafkreione/worker.js";
    const TWO: &str = "https://tonk.test/releases/bafkreitwo/worker.js";

    /// The head of `main` of the space under `key`.
    async fn head(tonk: &TonkState, key: &str) -> Revision {
        tonk.reactor
            .repository(key)
            .branch("main")
            .acquire(&tonk.operator)
            .await
            .expect("the space is mounted")
            .handle()
            .revision()
            .expect("the space has a head")
    }

    /// What is written where the shell reads its origin's pin.
    async fn written() -> serde_json::Value {
        let caches: CacheStorage =
            js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("caches"))
                .unwrap()
                .dyn_into()
                .unwrap();
        let cache: Cache = JsFuture::from(caches.open("tonk-boot"))
            .await
            .unwrap()
            .dyn_into()
            .unwrap();
        let held = JsFuture::from(cache.match_with_str("/__boot"))
            .await
            .unwrap();
        match held.dyn_into::<Response>() {
            Ok(held) => {
                serde_wasm_bindgen::from_value(JsFuture::from(held.json().unwrap()).await.unwrap())
                    .unwrap()
            }
            Err(_) => serde_json::Value::Null,
        }
    }

    #[dialog_common::test]
    async fn it_sets_a_signed_pin_on_the_head_that_later_commits_carry() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let key = put_repo(&app, "boot-pin").await;
        let tonk = state.read().await;
        let before = head(&tonk, &key).await;
        assert_eq!(before.boot, None, "a space starts with no pin");

        let boot = pin(&tonk, &key, "main", SRC, ONE)
            .await
            .expect("the pin is set");
        assert_eq!(boot.seq, 1);
        assert_eq!(boot.issuer, tonk.operator.did());

        let pinned = head(&tonk, &key).await;
        pinned.verify().expect("the head is still its issuer's");
        assert_ne!(
            pinned.tree, before.tree,
            "the pin is recorded in the branch"
        );
        assert_eq!(
            stated(&pinned),
            Some(&boot),
            "and stated on its head, signed"
        );
        assert_eq!(record(&pinned).unwrap()["pin"], ONE);

        // A second pin is a later statement.
        let next = pin(&tonk, &key, "main", SRC, TWO)
            .await
            .expect("the pin moves");
        assert_eq!(next.seq, 2);
        assert_eq!(record(&head(&tonk, &key).await).unwrap()["pin"], TWO);
    }

    #[dialog_common::test]
    async fn it_ignores_a_statement_that_is_not_its_issuers() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let key = put_repo(&app, "boot-forged").await;
        let tonk = state.read().await;
        pin(&tonk, &key, "main", SRC, ONE)
            .await
            .expect("the pin is set");

        let mut forged = head(&tonk, &key).await;
        forged.boot.as_mut().unwrap().pin = Some(TWO.to_owned());
        assert_eq!(stated(&forged), None);
        assert_eq!(record(&forged), None);
    }

    #[dialog_common::test]
    async fn it_writes_down_the_latest_pin_and_never_an_earlier_one() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let key = put_repo(&app, "boot-mirror").await;
        let tonk = state.read().await;

        let bare = head(&tonk, &key).await;
        mirror(&bare).await;
        assert_eq!(
            written().await,
            serde_json::Value::Null,
            "a head that states nothing writes nothing"
        );

        pin(&tonk, &key, "main", SRC, ONE)
            .await
            .expect("the pin is set");
        let first = head(&tonk, &key).await;
        mirror(&first).await;
        assert_eq!(written().await["pin"], ONE);

        pin(&tonk, &key, "main", SRC, TWO)
            .await
            .expect("the pin moves");
        let second = head(&tonk, &key).await;
        mirror(&second).await;
        assert_eq!(written().await["pin"], TWO);
        assert_eq!(written().await["src"], SRC);

        // An earlier statement, or a head with none, changes nothing.
        mirror(&first).await;
        mirror(&bare).await;
        assert_eq!(written().await["pin"], TWO);
    }
}
