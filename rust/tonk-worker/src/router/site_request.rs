//! The FABB's own acts, as commands: record the request on the asking
//! tab's site, for the bar that tab shows to act on.
//!
//! Opening a panel, the account ceremony and the clipboard are page
//! capabilities, so a handler cannot perform them. What it can do is say,
//! as facts, what the tab was asked to do: `xyz.tonk.site/request` (which
//! act) and `xyz.tonk.site/request-time` (when), on `site:<client>` in the
//! profile's session overlay, next to the tab's route stamp. The bar's
//! `<ui-site-request>` subscribes to its own site and, on a newer request,
//! presses the matching control. Overlay facts, so nothing is stored; they
//! go when the tab navigates or closes, with the rest of its site.

use dialog_artifacts::{Entity, Value};
use tonk_common::log;

use crate::router::CommandEnv;
use crate::router::claim::RawClaim;

/// Record `request` on the asking tab's site.
async fn ask(env: &CommandEnv, request: &str, time: f64) {
    let Some(client) = env.client() else {
        log!("site request {request}: no originating tab");
        return;
    };
    let Ok(site) = format!("site:{}", client.0).parse::<Entity>() else {
        return;
    };
    let claim = |name: &str, is: Value| {
        Some(RawClaim {
            the: format!("xyz.tonk.site/{name}").parse().ok()?,
            of: site.clone(),
            is,
            policy: dialog_artifacts::Policy::Last,
        })
    };
    let (Some(request_claim), Some(time_claim)) = (
        claim("request", Value::String(request.to_owned())),
        claim("request-time", Value::Float(time)),
    ) else {
        return;
    };
    let tonk = env.state().read().await;
    if let Err(error) = tonk
        .reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .overlay()
        .assert(request_claim)
        .assert(time_claim)
        .write()
        .perform(&tonk.operator)
        .await
    {
        log!("site request {request}: overlay write failed: {error}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::AddAccount> for CommandEnv {
    async fn execute(&self, command: tonk_schema::command::AddAccount) {
        ask(self, "account", command.time.0).await;
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::ShareLink> for CommandEnv {
    async fn execute(&self, command: tonk_schema::command::ShareLink) {
        ask(self, "share", command.time.0).await;
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::ViewMembers> for CommandEnv {
    async fn execute(&self, command: tonk_schema::command::ViewMembers) {
        ask(self, "members", command.time.0).await;
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::ConnectAgent> for CommandEnv {
    async fn execute(&self, command: tonk_schema::command::ConnectAgent) {
        ask(self, "agent", command.time.0).await;
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::ConnectSpace> for CommandEnv {
    async fn execute(&self, command: tonk_schema::command::ConnectSpace) {
        ask(self, "connect", command.time.0).await;
    }
}

/// Not one of the bar's controls: the introspection overlay in the frame
/// whose site this is answers it, and relays a pick to the frames inside,
/// so the frame that holds the displays is the one that opens.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::Inspect> for CommandEnv {
    async fn execute(&self, command: tonk_schema::command::Inspect) {
        ask(self, "inspect", command.time.0).await;
    }
}
