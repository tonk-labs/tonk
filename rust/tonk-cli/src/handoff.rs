//! Prototype agent connection acknowledgement, replicated as ordinary space data.

use crate::{eval, site::TonkSite};

/// Record a successful pull. The caller must push before reporting completion.
/// This is a durable acknowledgement, not a heartbeat or online-status claim.
pub async fn record_connection(site: &TonkSite) -> Result<eval::Outcome, eval::EvalError> {
    eval::run_against_site(
        site,
        eval::Source::Inline(
            "agent-connection!:\n  this: id:tonk:agent-connection\n  status: \"Agent connection confirmed\"\n".into(),
        ),
        eval::Options::default(),
    )
    .await
}

/// Pull the selected space and publish its receipt without waiting on the account directory.
/// Safe to repeat after a caller timeout: the receipt has a stable entity and value.
pub async fn confirm_connection(site: &TonkSite) -> anyhow::Result<()> {
    use anyhow::Context as _;
    crate::sync::pull(site)
        .await
        .context("space joined, but connection not confirmed: pull failed")?;
    record_connection(site).await?;
    crate::sync::push(site)
        .await
        .context("connection receipt is local; retry connect on this space to publish it")?;
    Ok(())
}

/// Choose a local alias from the pulled space's own name, without renaming the space.
pub async fn synced_name(
    site: &TonkSite,
    registry: &crate::space::Registry,
) -> anyhow::Result<String> {
    use anyhow::Context as _;
    use dialog_query::{Output as _, Query, Term};
    use tonk_schema::{RepositoryName, prelude::DidExt as _};
    let branch = site.branch().await?;
    let rows = branch
        .handle()
        .query()
        .select(Query::<RepositoryName> {
            this: Term::from(site.repository.did().this()),
            name: Term::var("name"),
        })
        .perform(&site.operator)
        .try_vec()
        .await?;
    let display_name = &rows
        .first()
        .context("the space has no synced name; retry with --name")?
        .name
        .0;
    Ok(available_name(display_name, registry))
}

fn available_name(display_name: &str, registry: &crate::space::Registry) -> String {
    let lowered = display_name.to_ascii_lowercase();
    let stem = lowered
        .split(|c: char| !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let stem = stem.trim_start_matches('_');
    let stem = if stem.is_empty() { "space" } else { stem };
    let mut name = stem.to_owned();
    let mut suffix = 2;
    while registry.spaces.contains_key(&name) {
        name = format!("{stem}-{suffix}");
        suffix += 1;
    }
    name
}
