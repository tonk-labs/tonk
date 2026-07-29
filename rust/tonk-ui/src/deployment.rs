//! Cached same-origin deployment configuration.

use std::cell::RefCell;

use futures_util::FutureExt;
use futures_util::future::{LocalBoxFuture, Shared};
use tonk_worker_api::DeploymentConfig;

type ConfigFuture = Shared<LocalBoxFuture<'static, Result<DeploymentConfig, String>>>;

thread_local! {
    static CONFIG: RefCell<Option<ConfigFuture>> = const { RefCell::new(None) };
}

async fn fetch() -> Result<DeploymentConfig, String> {
    let origin = web_sys::window()
        .and_then(|window| window.location().origin().ok())
        .ok_or_else(|| "window origin is unavailable".to_string())?;
    let response = reqwest::Client::new()
        .get(format!("{origin}/.well-known/tonk"))
        .send()
        .await
        .map_err(|_| "deployment configuration is unavailable".to_string())?;
    if !response.status().is_success() {
        return Err("deployment configuration is unavailable".to_string());
    }
    response
        .json()
        .await
        .map_err(|_| "deployment configuration is invalid".to_string())
}

/// Load this page deployment's service endpoints once.
pub(crate) async fn get() -> Result<DeploymentConfig, String> {
    let future = CONFIG.with(|slot| {
        slot.borrow_mut()
            .get_or_insert_with(|| fetch().boxed_local().shared())
            .clone()
    });
    future.await
}
