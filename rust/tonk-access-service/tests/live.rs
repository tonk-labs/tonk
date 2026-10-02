//! A space's live side, end to end in the production Wasm worker: a watch
//! begun over the space's socket is told of a cell write made over HTTP.
#![cfg(all(feature = "helpers", not(target_arch = "wasm32")))]

use dialog_capability::access::{Authorization as _, TimeRange};
use dialog_capability::{Ability, Capability, Effect, Subject};
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_effects::memory::prelude::CellScope;
use dialog_remote_ucan::UcanAuthorization;
use dialog_remote_ucan::socket::Request;
use dialog_ucan::Scope;
use dialog_ucan_core::Container;
use dialog_varsig::Principal as _;

/// The invocation `signer` mints for `capability` on its own authority, as
/// dialog's client mints it: saying when it was issued.
async fn issued<Fx>(signer: &Ed25519Signer, capability: &Capability<Fx>) -> UcanAuthorization
where
    Fx: Effect + Clone,
    Capability<Fx>: Ability,
{
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default();
    let minting = dialog_ucan::UcanAuthorization {
        chain: None,
        signer: Signer::from(signer.clone()),
        scope: Scope::invoke(capability),
        duration: TimeRange {
            not_before: Some(at),
            expiration: Some(at + 60),
        },
        meta: None,
    };
    UcanAuthorization::from(minting.invoke().await.expect("the invocation mints"))
}

/// Opt-in because it executes the freshly built production Wasm worker in
/// workerd. `scripts/test-live-worker.sh` supplies its shim and Miniflare.
#[tokio::test]
#[ignore = "requires worker-build and local Miniflare; run scripts/test-live-worker.sh"]
async fn live_worker_tells_a_watch_of_a_write_over_http() -> anyhow::Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let shim = std::env::var("TONK_LIVE_WORKER_SHIM")?;

    let space = Ed25519Signer::generate().await?;
    let cell = CellScope::new(Subject::from(space.did()), "branch/main", "revision");
    let content = b"a head another device published".to_vec();

    let watch = issued(&space, &cell.watch()).await;
    let frame = Request::Invoke {
        container: Container::from(watch.invocation().chain()).to_bytes()?,
        payload: None,
    }
    .encode();
    let publish = issued(&space, &cell.publish(content.clone(), None)).await;
    let credential = dialog_remote_ucan::credential(Container::from(publish.invocation().chain()))?;

    let state = tempfile::tempdir()?;
    let fixture = state.path().join("fixture.json");
    std::fs::write(
        &fixture,
        serde_json::to_vec(&serde_json::json!({
            "subject": space.did().to_string(),
            "watch": frame,
            "publish": credential,
            "content": content,
        }))?,
    )?;
    let result = std::process::Command::new("node")
        .arg(root.join("scripts/live-worker.cjs"))
        .arg(&shim)
        .arg(fixture)
        .arg(&root)
        .arg(state.path())
        .status()?;
    anyhow::ensure!(
        result.success(),
        "production worker live harness failed: {result}"
    );
    Ok(())
}
