//! `site/select`: record what the page in a tab has selected, on the tab's
//! site.
//!
//! The page (`bootstrap.js`, in every guest) reports its selection as it
//! settles. The handler writes it as `xyz.tonk.site/selection` on the tab's
//! site, in the session overlay, so rules and the palette read it like any
//! other site fact: rules that run on a space branch next to the space's
//! data, and the palette's parser through the expression. It is written on
//! the branch the report came from and on the profile's, because the
//! palette proposes commands from both. Overlay facts only: nothing is
//! stored, and the selection goes with the tab.

use dialog_artifacts::{Change, Entity, Value};
use tonk_common::log;
use tonk_schema::invocation::{Done, Failure, Outcome};

use crate::router::CommandEnv;
use crate::router::claim::RawClaim;

/// The site attribute the selection is recorded under.
const SELECTION: &str = "xyz.tonk.site/selection";

/// The most of a selection kept, in characters. A selection is a hint for
/// what a command could act on, not content to copy, so a long one is cut.
const LIMIT: usize = 4096;

/// Replace `site`'s selection with `text` on one branch's overlay, or clear
/// it when `text` is empty. The error says, for a person, why it was not
/// recorded.
async fn record(
    tonk: &crate::worker::TonkState,
    branch: dialog_reactor::BranchReference<'_>,
    site: &Entity,
    text: &str,
) -> Result<(), Failure> {
    let Ok(the) = SELECTION.parse() else {
        return Err(Failure::new("the selection attribute does not parse"));
    };
    let session = match branch.acquire(&tonk.operator).await {
        Ok(session) => session,
        Err(error) => {
            log!("site/select: branch unavailable: {error}");
            return Err(Failure::new(format!("the branch is unavailable: {error}")));
        }
    };
    // A cardinality-one fact is cleared by retracting the value it holds,
    // so read what the overlay has now.
    let held: Vec<Value> = session
        .handle()
        .overlay()
        .export()
        .iter()
        .filter(|(entity, attribute, _)| *entity == site && attribute.to_string() == SELECTION)
        .filter_map(|(_, _, change)| match change {
            Change::Assert(value, _) => Some(value.clone()),
            Change::Retract(_) => None,
        })
        .collect();
    let mut overlay = branch.overlay();
    for value in held {
        overlay = overlay.retract(RawClaim {
            the: SELECTION.parse().expect("the selection attribute parses"),
            of: site.clone(),
            is: value,
            policy: dialog_artifacts::Pick::Last,
        });
    }
    if !text.is_empty() {
        overlay = overlay.assert(RawClaim {
            the,
            of: site.clone(),
            is: Value::String(text.to_owned()),
            policy: dialog_artifacts::Pick::Last,
        });
    }
    if let Err(error) = overlay.write().perform(&tonk.operator).await {
        log!("site/select: overlay write failed: {error}");
        return Err(Failure::new(format!(
            "the selection could not be recorded: {error}"
        )));
    }
    Ok(())
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::SiteSelect> for CommandEnv {
    async fn execute(&self, command: tonk_schema::command::SiteSelect) -> Outcome {
        let site = command.site.0;
        if !site.to_string().starts_with("site:") {
            log!("site/select: not a site: {site}");
            return Err(Failure::new(format!("{site} is not a site")));
        }
        let text: String = command.text.0.trim().chars().take(LIMIT).collect();
        let tonk = self.state().read().await;
        let origin = self.origin();
        if origin.repo.is_empty() {
            let branch = tonk.reactor.profile_repository().branch(&origin.branch);
            record(&tonk, branch, &site, &text).await?;
        } else {
            let branch = tonk.reactor.repository(&origin.repo).branch(&origin.branch);
            // Both branches are written even when the first fails, as
            // before; the outcome is the first failure.
            let space = record(&tonk, branch, &site, &text).await;
            let profile = tonk
                .reactor
                .profile_repository()
                .branch(&tonk.active_branch);
            let profile = record(&tonk, profile, &site, &text).await;
            space.and(profile)?;
        }
        Done::with(site)
    }
}

#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
mod tests {
    use crate::router::command::tests::native::test_state;
    use crate::router::{CommandEnv, CommandOrigin};
    use tonk_schema::domain::command::current::site_select::{Site, Text, Time};

    /// A selection reported against an entity that is not a site is refused
    /// with an outcome saying so, and nothing is recorded on the profile
    /// branch's overlay.
    #[dialog_common::test]
    async fn it_refuses_a_selection_on_something_that_is_not_a_site() {
        let state = test_state().await;
        let branch = state.read().await.active_branch.clone();
        let env = CommandEnv::new(
            state.clone(),
            CommandOrigin {
                branch: branch.clone(),
                ..Default::default()
            },
        );
        let command = tonk_schema::command::SiteSelect {
            this: "id:select".parse().unwrap(),
            site: Site("id:not-a-site".parse().unwrap()),
            text: Text("hello".into()),
            time: Time(0.0),
        };
        let outcome =
            dialog_capability::Provider::<tonk_schema::command::SiteSelect>::execute(&env, command)
                .await;
        assert!(matches!(&outcome, Err(failure) if failure.0.contains("not a site")));

        let tonk = state.read().await;
        let session = tonk
            .reactor
            .profile_repository()
            .branch(&branch)
            .acquire(&tonk.operator)
            .await
            .unwrap();
        let recorded = session
            .handle()
            .overlay()
            .export()
            .iter()
            .any(|(_, attribute, _)| attribute.to_string() == super::SELECTION);
        assert!(!recorded, "a refused selection must not be recorded");
    }
}
