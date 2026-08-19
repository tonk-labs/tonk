//! Worker adapter for hosted-space deletion.

#[cfg(target_arch = "wasm32")]
pub async fn handle(body: &[u8], env: &worker::Env) -> worker::Result<worker::Response> {
    let store = crate::store::d1::D1Store::new(env.d1("CONTROL")?);
    let purger = crate::deletion::R2SpacePurger::new(env.bucket("BUCKET")?);
    let now = worker::Date::now().as_millis() / 1_000;
    match crate::deletion::delete(&store, &purger, body, now).await {
        Ok(receipt) => worker::Response::from_json(&receipt),
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
        crate::deletion::delete_customer(&store, &purger, body, now)
            .await
            .map(|receipt| serde_json::to_value(receipt).expect("receipt serializes"))
    };
    match result {
        Ok(receipt) => worker::Response::from_json(&receipt),
        Err(error) => worker::Response::from_json(&serde_json::json!({ "error": error }))
            .map(|response| response.with_status(error.status())),
    }
}
