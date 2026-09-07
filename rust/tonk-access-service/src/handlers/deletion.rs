//! Worker adapter for hosted-space deletion.

/// Deprovisioning deletes the cached servability verdict rather than
/// writing a negative one, forcing the next presign through
/// authoritative D1 while the purge is in flight. Best-effort: a
/// missed delete rides out the verdict's own validity.
#[cfg(target_arch = "wasm32")]
async fn forget_verdict(subject: &str, env: &worker::Env) {
    use crate::provisioning::cache;

    cache::isolate_forget(subject);
    if let Some(kv) = crate::handlers::ucan::servability_kv(env)
        && let Err(err) = kv.delete(&cache::key(subject)).await
    {
        worker::console_error!("cached verdict for {subject} not dropped: {err}");
    }
}

#[cfg(target_arch = "wasm32")]
pub async fn handle(body: &[u8], env: &worker::Env) -> worker::Result<worker::Response> {
    let store = crate::store::d1::D1Store::new(env.d1("CONTROL")?);
    let purger = crate::deletion::R2SpacePurger::new(env.bucket("BUCKET")?);
    let now = worker::Date::now().as_millis() / 1_000;
    match crate::deletion::delete(&store, &purger, body, now).await {
        Ok(receipt) => {
            forget_verdict(receipt.space.as_str(), env).await;
            worker::Response::from_json(&receipt)
        }
        Err(error) => worker::Response::from_json(&serde_json::json!({ "error": error }))
            .map(|response| response.with_status(error.status())),
    }
}

/// Handle the customer purge.
#[cfg(target_arch = "wasm32")]
pub async fn handle_purge(body: &[u8], env: &worker::Env) -> worker::Result<worker::Response> {
    let store = crate::store::d1::D1Store::new(env.d1("CONTROL")?);
    let purger = crate::deletion::R2SpacePurger::new(env.bucket("BUCKET")?);
    let now = worker::Date::now().as_millis() / 1_000;
    match crate::deletion::purge(&store, &purger, body, now).await {
        Ok(receipt) => {
            for consumer in &receipt.consumers {
                forget_verdict(consumer, env).await;
            }
            // The customer-row replica goes with it; the address key is
            // unreachable from the DID once the row is gone and rides
            // out its own validity instead.
            if let Some(kv) = crate::handlers::ucan::servability_kv(env) {
                crate::store::replica::forget(&kv, &receipt.customer).await;
            }
            worker::Response::from_json(&receipt)
        }
        Err(error) => worker::Response::from_json(&serde_json::json!({ "error": error }))
            .map(|response| response.with_status(error.status())),
    }
}
