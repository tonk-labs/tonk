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
use tonk_schema::invocation::{Done, Failure, Outcome};

use crate::router::CommandEnv;
use crate::router::claim::RawClaim;

/// Record `request` on the asking tab's site, and say which site it was
/// recorded on. The bar performs the request itself once it sees it, so
/// whether that act succeeds is not part of this outcome.
async fn ask(env: &CommandEnv, request: &str, time: f64) -> Outcome {
    let Some(client) = env.client() else {
        log!("site request {request}: no originating tab");
        return Err(Failure::new(format!(
            "There is no tab to ask for {request}."
        )));
    };
    let Ok(site) = format!("site:{}", client.0).parse::<Entity>() else {
        return Err(Failure::new(format!(
            "The asking tab has no usable site for {request}."
        )));
    };
    let claim = |name: &str, is: Value| {
        Some(RawClaim {
            the: format!("xyz.tonk.site/{name}").parse().ok()?,
            of: site.clone(),
            is,
            policy: dialog_artifacts::Pick::Last,
        })
    };
    let (Some(request_claim), Some(time_claim)) = (
        claim("request", Value::String(request.to_owned())),
        claim("request-time", Value::Float(time)),
    ) else {
        return Err(Failure::new(format!(
            "Could not describe the {request} request."
        )));
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
        return Err(Failure::new(format!(
            "Could not record the {request} request: {error}"
        )));
    }
    Done::with(site)
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::AddAccount> for CommandEnv {
    async fn execute(&self, command: tonk_schema::command::AddAccount) -> Outcome {
        ask(self, "account", command.time.0).await
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::ShareLink> for CommandEnv {
    async fn execute(&self, command: tonk_schema::command::ShareLink) -> Outcome {
        ask(self, "share", command.time.0).await
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::ViewMembers> for CommandEnv {
    async fn execute(&self, command: tonk_schema::command::ViewMembers) -> Outcome {
        ask(self, "members", command.time.0).await
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::ConnectAgent> for CommandEnv {
    async fn execute(&self, command: tonk_schema::command::ConnectAgent) -> Outcome {
        ask(self, "agent", command.time.0).await
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::ConnectSpace> for CommandEnv {
    async fn execute(&self, command: tonk_schema::command::ConnectSpace) -> Outcome {
        ask(self, "connect", command.time.0).await
    }
}

/// Not one of the bar's controls: the introspection overlay in the frame
/// whose site this is answers it, and relays a pick to the frames inside,
/// so the frame that holds the displays is the one that opens.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::Inspect> for CommandEnv {
    async fn execute(&self, command: tonk_schema::command::Inspect) -> Outcome {
        ask(self, "inspect", command.time.0).await
    }
}

#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
mod tests {
    use super::*;
    use crate::router::CommandOrigin;
    use crate::router::command::tests::native::test_state;

    fn add_account() -> tonk_schema::command::AddAccount {
        tonk_schema::command::AddAccount {
            this: "command:add-account-test".parse().unwrap(),
            time: tonk_schema::domain::command::current::add_account::Time(1.0),
        }
    }

    /// A request with no tab to ask fails, saying so, rather than
    /// vanishing.
    #[dialog_common::test]
    async fn a_request_without_a_tab_fails() {
        let state = test_state().await;
        let env = CommandEnv::new(state.clone(), CommandOrigin::default());
        let outcome = dialog_capability::Provider::<tonk_schema::command::AddAccount>::execute(
            &env,
            add_account(),
        )
        .await;
        assert!(matches!(&outcome, Err(failure) if failure.0.contains("no tab")));
    }

    /// A request recorded on the asking tab's site names that site.
    #[dialog_common::test]
    async fn a_request_names_the_site_it_was_recorded_on() {
        let state = test_state().await;
        let origin = CommandOrigin {
            client: Some(crate::router::ClientId("tab-1".to_owned())),
            ..CommandOrigin::default()
        };
        let env = CommandEnv::new(state.clone(), origin);
        let outcome = dialog_capability::Provider::<tonk_schema::command::AddAccount>::execute(
            &env,
            add_account(),
        )
        .await;
        let site: Entity = "site:tab-1".parse().unwrap();
        assert_eq!(outcome, Done::with(site));
    }
}
