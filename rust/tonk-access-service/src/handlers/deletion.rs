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

/// Handle root-authenticated customer inventory and finalization commands.
#[cfg(target_arch = "wasm32")]
pub async fn handle_customer(body: &[u8], env: &worker::Env) -> worker::Result<worker::Response> {
    let store = crate::store::d1::D1Store::new(env.d1("CONTROL")?);
    let purger = crate::deletion::R2SpacePurger::new(env.bucket("BUCKET")?);
    let now = worker::Date::now().as_millis() / 1_000;
    let result = if crate::deletion::command_for_handler(body)
        == crate::deletion::CUSTOMER_PLAN_COMMAND.map(str::to_string)
    {
        crate::deletion::customer_plan(&store, body, now)
            .await
            .map(|plan| serde_json::to_value(plan).expect("plan serializes"))
    } else {
        match crate::deletion::delete_customer(&store, &purger, body, now).await {
            Ok(receipt) => {
                forget_verdict(&receipt.customer, env).await;
                Ok(serde_json::to_value(receipt).expect("receipt serializes"))
            }
            Err(error) => Err(error),
        }
    };
    match result {
        Ok(receipt) => worker::Response::from_json(&receipt),
        Err(error) => worker::Response::from_json(&serde_json::json!({ "error": error }))
            .map(|response| response.with_status(error.status())),
    }
}
