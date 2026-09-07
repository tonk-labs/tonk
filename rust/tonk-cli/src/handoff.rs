//! Agent connection acknowledgement, replicated as ordinary space data.

use crate::{eval, site::TonkSite};

/// Select the browser page for account approval without trusting invite data.
/// An explicit override is a user decision; otherwise use Tonk's production
/// account page.
pub fn approval_page(explicit: Option<&str>) -> anyhow::Result<String> {
    use anyhow::Context as _;
    let mut page = match explicit {
        Some(explicit) => url::Url::parse(explicit).context("--via is not a valid URL")?,
        None => url::Url::parse(crate::account::DEFAULT_LINK_PAGE)
            .expect("the built-in account page is a valid URL"),
    };
    anyhow::ensure!(
        matches!(page.scheme(), "http" | "https") && page.host_str().is_some(),
        "the account approval page must use HTTP or HTTPS"
    );
    let _ = page.set_username("");
    let _ = page.set_password(None);
    page.set_fragment(None);
    Ok(page.to_string())
}

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
    let stem = stem.trim_start_matches(|c: char| !c.is_ascii_alphanumeric());
    let stem = if stem.is_empty() { "space" } else { stem };
    let mut name = stem.to_owned();
    let mut suffix = 2;
    while registry.spaces.contains_key(&name) {
        name = format!("{stem}-{suffix}");
        suffix += 1;
    }
    name
}

/// Produce a stable, resumable local name when the remote display name is not
/// available yet. The repository DID, not time or local process state, is the
/// input so a person can recognize retries for the same space.
pub fn fallback_name(subject: &str, registry: &crate::space::Registry) -> String {
    let identifier = subject.rsplit(':').next().unwrap_or(subject);
    let short = identifier.chars().take(8).collect::<String>();
    available_name(&format!("space-{short}"), registry)
}

/// Result of inspecting registered spaces for an exact invitation claim.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct InvitationMatch {
    /// Existing local alias that claimed this exact invitation.
    pub name: Option<String>,
    /// Registry entries that could not be inspected.
    pub diagnostics: Vec<String>,
}

/// Find an already-registered replica created from this exact invitation.
///
/// A fresh invitation to the same repository may carry newer authority, so a
/// repository-DID match is deliberately insufficient. Unrelated damaged
/// entries are reported but do not make every new connection unavailable.
/// Older replicas without local claim metadata must resume by explicit alias.
pub async fn matching_invitation(
    registry: &crate::space::Registry,
    config: &crate::site::SiteConfig,
    invitation: &tonk_schema::Invitation,
) -> InvitationMatch {
    use tonk_schema::prelude::DidExt as _;
    let mut result = InvitationMatch::default();
    for (name, entry) in &registry.spaces {
        let site = match crate::site::TonkSite::open_with(&entry.site, config.clone()).await {
            Ok(site) => site,
            Err(error) => {
                result.diagnostics.push(format!(
                    "could not inspect registered space '{name}': {error:#}"
                ));
                continue;
            }
        };
        if site.repository.did().this() != invitation.subject.0 {
            continue;
        }
        // Roster rows and branch delegations can arrive through replication.
        // Only the local claim marker proves this replica installed the invite.
        match std::fs::read_to_string(site.root.join(crate::invite::CLAIMED_INVITATION_FILE)) {
            Ok(claimed) if claimed == invitation.this.to_string() => {
                result.name = Some(name.clone());
                return result;
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => result.diagnostics.push(format!(
                "could not inspect registered space '{name}': {error:#}"
            )),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_uses_the_builtin_production_page_by_default() {
        assert_eq!(
            approval_page(None).unwrap(),
            crate::account::DEFAULT_LINK_PAGE
        );
    }

    #[test]
    fn approval_accepts_an_explicit_local_override_but_rejects_other_schemes() {
        assert_eq!(
            approval_page(Some("http://127.0.0.1:8080/settings/link")).unwrap(),
            "http://127.0.0.1:8080/settings/link"
        );
        assert!(approval_page(Some("file:///tmp/settings/link")).is_err());
    }

    #[test]
    fn generated_names_start_with_an_alphanumeric_character() {
        let registry = crate::space::Registry::default();
        for (display, expected) in [
            ("_ Garden", "garden"),
            ("_ - _ Garden", "garden"),
            ("_ - _", "space"),
            ("", "space"),
            ("日本語", "space"),
            ("_ 2 Gardens", "2-gardens"),
            ("Garden_Bed", "garden_bed"),
        ] {
            let name = available_name(display, &registry);
            assert_eq!(name, expected, "{display}");
            crate::space::validate_name(&name).unwrap();
        }
    }

    #[test]
    fn fallback_names_are_stable_and_collision_safe() {
        let mut registry = crate::space::Registry::default();
        assert_eq!(
            fallback_name("did:key:z6MkExampleSubject", &registry),
            "space-z6mkexam"
        );
        registry.spaces.insert(
            "space-z6mkexam".into(),
            crate::space::SpaceEntry::at("/tmp/example"),
        );
        assert_eq!(
            fallback_name("did:key:z6MkExampleSubject", &registry),
            "space-z6mkexam-2"
        );
    }
}
