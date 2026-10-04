//! `site/select`: record what the page in a tab has selected, on the tab's
//! site.
//!
//! The page (`bootstrap.js`, in every guest) reports its selection as it
//! settles. The handler writes it as `xyz.tonk.site/selection` on the tab's
//! site, in the branch's state layer, so rules and the palette read it like any
//! other site fact: rules that run on a space branch next to the space's
//! data, and the palette's parser through the expression. It is written on
//! the branch the report came from and on the profile's, because the
//! palette proposes commands from both. State-layer facts only: nothing is
//! stored, and the selection goes with the tab.

use dialog_artifacts::{Change, Entity, Value};
use tonk_common::log;

use crate::router::CommandEnv;
use crate::router::claim::RawClaim;

/// The site attribute the selection is recorded under.
const SELECTION: &str = "xyz.tonk.site/selection";

/// The most of a selection kept, in characters. A selection is a hint for
/// what a command could act on, not content to copy, so a long one is cut.
const LIMIT: usize = 4096;

/// Replace `site`'s selection with `text` on one branch's state layer, or clear
/// it when `text` is empty.
async fn record(
    tonk: &crate::worker::TonkState,
    branch: dialog_reactor::BranchReference<'_>,
    site: &Entity,
    text: &str,
) {
    let Ok(the) = SELECTION.parse() else {
        return;
    };
    let session = match branch.acquire(&tonk.operator).await {
        Ok(session) => session,
        Err(error) => {
            log!("site/select: branch unavailable: {error}");
            return;
        }
    };
    // A cardinality-one fact is cleared by retracting the value it holds,
    // so read what the state layer has now.
    let held: Vec<Value> = session
        .state
        .state_layer()
        .export()
        .iter()
        .filter(|(entity, attribute, _)| *entity == site && attribute.to_string() == SELECTION)
        .filter_map(|(_, _, change)| match change {
            Change::Assert(value) | Change::Replace(value) => Some(value.clone()),
            Change::Retract(_) => None,
        })
        .collect();
    let mut overlay = branch.overlay();
    for value in held {
        overlay = overlay.retract(RawClaim {
            the: SELECTION.parse().expect("the selection attribute parses"),
            of: site.clone(),
            is: value,
            unique: true,
        });
    }
    if !text.is_empty() {
        overlay = overlay.assert(RawClaim {
            the,
            of: site.clone(),
            is: Value::String(text.to_owned()),
            unique: true,
        });
    }
    if let Err(error) = overlay.write().perform(&tonk.operator).await {
        log!("site/select: overlay write failed: {error}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::SiteSelect> for CommandEnv {
    async fn execute(&self, command: tonk_schema::command::SiteSelect) {
        let site = command.site.0;
        if !site.to_string().starts_with("site:") {
            log!("site/select: not a site: {site}");
            return;
        }
        let text: String = command.text.0.trim().chars().take(LIMIT).collect();
        let tonk = self.state().read().await;
        let origin = self.origin();
        if origin.repo.is_empty() {
            let branch = tonk.reactor.profile_repository().branch(&origin.branch);
            record(&tonk, branch, &site, &text).await;
        } else {
            let branch = tonk.reactor.repository(&origin.repo).branch(&origin.branch);
            record(&tonk, branch, &site, &text).await;
            let profile = tonk
                .reactor
                .profile_repository()
                .branch(&tonk.active_branch);
            record(&tonk, profile, &site, &text).await;
        }
    }
}
