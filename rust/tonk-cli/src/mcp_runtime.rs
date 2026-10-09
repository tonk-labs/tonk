//! Transport-independent tool execution for a host-selected native Tonk site.
//! Own the runtime exclusively: calls are serialized through `&mut self`.

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::{eval, site::TonkSite};

/// A single authorized native replica. Authentication and site selection belong
/// to the hosting service, never to model-provided tool arguments.
pub struct Runtime {
    site: TonkSite,
}

impl Runtime {
    /// Open exactly one host-selected account space using explicit tenant storage.
    /// Requires a successful pull and signed membership, including for cached
    /// local registrations. This is host control, not a model-callable tool.
    /// Callers must also enforce their deployment's outbound-network policy.
    pub async fn open_account_space(
        config: &crate::site::SiteConfig,
        subject: &dialog_varsig::Did,
    ) -> Result<Self> {
        if !config.require_account {
            bail!("Account space execution requires account-bound authority.");
        }
        let selected =
            crate::account_spaces::pull_with_config(config, subject.as_str(), None).await?;
        let site = TonkSite::open_with(&selected.site, config.clone()).await?;
        if site.repository.did() != *subject {
            bail!("Selected space does not match the requested subject.");
        }
        if selected.already_local {
            crate::sync::pull(&site).await?;
        }
        if !matches!(
            crate::inventory::role_for_site(&site).await?,
            crate::inventory::SpaceRole::Owner | crate::inventory::SpaceRole::Member
        ) {
            bail!("Selected space has no signed membership for this account profile.");
        }
        Ok(Self::new(site))
    }

    /// Bind to a site that the caller has already opened with explicit authority.
    pub fn new(site: TonkSite) -> Self {
        Self { site }
    }

    /// Explicit host-controlled upload. Never repeats the preceding evaluation.
    pub async fn push(&mut self) -> Result<crate::sync::SyncOutcome> {
        Ok(crate::sync::push(&self.site).await?)
    }

    /// Explicit host-controlled download. Conflicting histories remain an error.
    pub async fn pull(&mut self) -> Result<crate::sync::SyncOutcome> {
        Ok(crate::sync::pull(&self.site).await?)
    }

    /// Tools implemented by this host. Presentation is added by the MCP adapter.
    pub fn capabilities() -> &'static [&'static str] {
        &[
            "tonk_query",
            "tonk_preview",
            "tonk_apply",
            "tonk_space_info",
            "tonk_install_library",
        ]
    }

    /// Execute one canonical tool, preserving preview and uncertain-write semantics.
    pub async fn call(&mut self, name: &str, arguments: Value) -> Result<Value> {
        if name == "ui_read" {
            return crate::mcp_ui::read(&self.site, arguments).await;
        }
        let fields = arguments
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("Tool arguments must be an object."))?;
        if name == "tonk_install_library" {
            return crate::mcp_library::install(&self.site, &arguments).await;
        }
        if name == "tonk_space_info" {
            if !fields.is_empty() {
                bail!("This tool accepts no arguments or target space.");
            }
            return Ok(
                json!({"subject": self.site.repository.did().to_string(), "branches": ["main"]}),
            );
        }
        if !matches!(name, "tonk_query" | "tonk_preview" | "tonk_apply") {
            bail!("Tool unavailable.");
        }
        let apply = name == "tonk_apply";
        if fields.len() != if apply { 2 } else { 1 }
            || !fields.contains_key("document")
            || (apply && !fields.contains_key("expectedRevision"))
        {
            bail!("Provide only document and, for apply, expectedRevision.");
        }
        let document = fields["document"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Document must be inline notation."))?;
        // Match the desktop's bounded inline-only contract before the CLI's
        // evaluator can reach its filesystem include resolver.
        if document.trim().is_empty()
            || document.len() > 32_000
            || document.contains("%TAG")
            || document
                .match_indices('!')
                .any(|(i, _)| document.as_bytes().get(i + 1) != Some(&b':'))
        {
            bail!("Provide only inline notation, up to 32 KB, without YAML tags or includes.");
        }
        if apply {
            let expected = fields["expectedRevision"].clone();
            if serde_json::to_vec(&expected)?.len() > 16_000 {
                bail!("Invalid expectedRevision.");
            }
            let revision = serde_json::from_value(expected.clone())?;
            let response = match eval::run_conditional(&self.site, document.into(), revision).await
            {
                Ok(outcome) => outcome.response,
                Err(eval::EvalError::Io(_)) => {
                    bail!(
                        "The write outcome could not be confirmed. Do not repeat it; query the space first."
                    );
                }
                Err(error) => return Err(error.into()),
            };
            Ok(json!({
                "revision": response.revision_after,
                "previousRevision": response.revision_before,
                "revisionChanged": response.revision_before != response.revision_after,
                "claims": response.commits.claims,
                "accepted": true,
                "renderingConfirmed": false,
                "scope": "Local conditional evaluation completed. Rendering and remote synchronization are not confirmed. Query records and inspect the preview; do not repeat this write."
            }))
        } else {
            let response = eval::run_against_site(
                &self.site,
                eval::Source::Inline(document.into()),
                eval::Options {
                    dry_run: true,
                    ..Default::default()
                },
            )
            .await?
            .response;
            if serde_json::to_vec(&response.matches_after)?.len() > 100_000 {
                bail!(
                    "Too many results. Narrow the notation document. No partial results were returned."
                );
            }
            Ok(json!({
                "revision": response.revision_after,
                "committed": false,
                "matches": response.matches_after,
                "scope": "Local replica; current matches only. No sync, commit, rendered preview, or proposed-state diff."
            }))
        }
    }
}
