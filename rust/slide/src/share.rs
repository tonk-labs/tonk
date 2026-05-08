//! `slide share concept <name>` — push the local repo to its
//! upstream, mint an audience-open invite that embeds the
//! upstream's URL, and return a launcher URL the human can paste
//! into a browser.
//!
//! The launcher URL extends the standard invite URL with two
//! extra query parameters:
//!
//! - `name=<space-name>` — pre-fills the join form's "Local
//!   name" field. The human can rename before submitting.
//! - `then=<path-suffix>` — tells tonk-ui where to navigate
//!   after a successful claim, *relative to* the space's root.
//!   For a concept share this is `branch/main/concept/<name>`;
//!   tonk-ui prefixes `/space/<actual-name>/` using whatever
//!   local name the recipient ended up with (which can differ
//!   from `name=` — e.g. when the recipient already had the
//!   subject mounted under another name and lands in the
//!   already-member auto-claim path).
//!
//! `then=` degrades gracefully when tonk-ui doesn't yet honour
//! it: the join still completes, the human just lands on the
//! default post-claim page and navigates to the concept tile by
//! hand. A future tonk-ui change makes the navigation automatic.
//!
//! Slide does not invent a new write path for this — every byte
//! that lands on the branch came from `slide eval` or another
//! existing slide subcommand.

use thiserror::Error;
use url::Url;

use crate::ExitCode;
use crate::invite::{self, InviteError};
use crate::remote::{self, RemoteError, RemoteRecord};
use crate::schema;
use crate::site::{self, SlideSite};
use crate::sync::{self, SyncError};

/// Default local-name suggestion encoded into the launcher URL's
/// `name=` parameter. The join form pre-fills with this; the
/// human can rename before submitting. Plain enough to not
/// collide with whatever the human had in mind, but
/// recognisable as a slide-originated share.
pub const DEFAULT_SPACE_NAME: &str = "shared";

/// Per-call knobs for [`share_concept`]. All optional — the
/// defaults match the most common agent flow (one configured
/// remote, a "shared" space name, the workspace's standard UI
/// base).
#[derive(Debug, Default, Clone)]
pub struct ShareOptions {
    /// Override the URL prefix the invite is built against.
    /// Mirrors the same option on `slide invite`. `None` falls
    /// back to [`DEFAULT_BASE_URL`].
    pub ui_base: Option<String>,
    /// Explicit remote name to embed. `None` auto-selects the
    /// only registered remote and errors when there's zero or
    /// more than one.
    pub remote: Option<String>,
    /// Override the `name=` query parameter (the suggested local
    /// name on the join form). `None` defaults to
    /// [`DEFAULT_SPACE_NAME`].
    pub space_name: Option<String>,
}

/// Outcome of [`share_concept`].
#[derive(Debug)]
pub struct ShareOutcome {
    /// The launcher URL — base invite URL plus `name=` and
    /// `then=` query parameters.
    pub url: String,
    /// Local name of the remote whose endpoint got embedded as
    /// `remote=`. Echoed back for the CLI to print.
    pub remote_name: String,
    /// Endpoint URL, also echoed.
    pub remote_endpoint: String,
    /// Concept the URL targets — the shared canonical name.
    pub concept_name: String,
    /// `name=` value embedded into the URL.
    pub space_name: String,
}

/// Failure modes for [`share_concept`].
#[derive(Debug, Error)]
pub enum ShareError {
    /// `<name>` doesn't resolve to a concept on the local
    /// branch. Most often a typo; `slide concepts` lists what's
    /// available.
    #[error(
        "concept '{name}' is not defined on this branch; \
         run `slide concepts` to see what's available"
    )]
    ConceptNotFound {
        /// The name that didn't resolve.
        name: String,
    },
    /// No remote is registered. The share flow needs a remote so
    /// the joined site can pull from somewhere.
    #[error(
        "no remote is registered; add one with `slide remote add <name> <url>` \
         before sharing"
    )]
    NoRemote,
    /// Multiple remotes are registered and the caller didn't
    /// pick one with `--remote`. We don't auto-select in this
    /// case to avoid embedding the wrong endpoint.
    #[error(
        "multiple remotes registered; pass `--remote <name>` to choose. \
         Available: {0}"
    )]
    AmbiguousRemote(String),
    /// `--remote <name>` was supplied but no remote with that
    /// name is registered.
    #[error("remote '{0}' is not registered; run `slide remote list` to see what's there")]
    UnknownRemote(String),
    /// The local branch has no upstream configured. Without one,
    /// `slide push` would fail and the share would mint an
    /// invite the human can't actually pull from.
    #[error(
        "branch '{branch}' has no upstream configured; \
         run `slide remote set-upstream <remote>` first"
    )]
    UpstreamNotConfigured {
        /// Branch missing an upstream — always `main` for slide.
        branch: String,
    },
    /// `slide push` failed. Most commonly non-fast-forward when
    /// the upstream advanced; pull and retry.
    #[error("push failed: {0}")]
    PushFailed(SyncError),
    /// Mint failed (key generation, delegation build, URL
    /// serialization). Surfaced verbatim.
    #[error("invite mint failed: {0}")]
    MintFailed(InviteError),
    /// Catch-all for everything else — meta-branch reads,
    /// schema queries, URL parsing.
    #[error("{0}")]
    Io(String),
}

impl ShareError {
    /// CLI exit code for this failure mode.
    pub fn exit_code(&self) -> ExitCode {
        match self {
            ShareError::PushFailed(SyncError::NonFastForward) => ExitCode::CommitError,
            _ => ExitCode::IoError,
        }
    }
}

impl From<RemoteError> for ShareError {
    fn from(err: RemoteError) -> Self {
        match err {
            RemoteError::UnknownRemote(name) => ShareError::UnknownRemote(name),
            RemoteError::Io(message) => ShareError::Io(message),
        }
    }
}

/// Push the local repo, mint an audience-open invite over it,
/// and produce a launcher URL pointing at a concept view.
///
/// Pre-flight ordering matters: each step that can fail
/// independently runs before the side-effecting ones (push, mint)
/// so an early error doesn't leave the user with half-applied
/// state. Specifically:
///
/// 1. Verify the concept exists.
/// 2. Resolve the remote (no I/O beyond a meta-branch read).
/// 3. Verify the local branch has an upstream.
/// 4. Push.
/// 5. Mint the invite.
/// 6. Compose the launcher URL.
pub async fn share_concept(
    site: &SlideSite,
    concept_name: &str,
    options: ShareOptions,
) -> Result<ShareOutcome, ShareError> {
    let concepts = schema::list_concepts(site)
        .await
        .map_err(|e| ShareError::Io(format!("failed to list concepts: {e}")))?;
    if !concepts.iter().any(|c| c.name == concept_name) {
        return Err(ShareError::ConceptNotFound {
            name: concept_name.to_owned(),
        });
    }

    let remote_record = resolve_remote(site, options.remote.as_deref()).await?;

    if site.branch.upstream().is_none() {
        return Err(ShareError::UpstreamNotConfigured {
            branch: site::BRANCH_NAME.to_owned(),
        });
    }

    sync::push(site).await.map_err(ShareError::PushFailed)?;

    let invite_outcome = invite::mint(
        site,
        options.ui_base.as_deref(),
        Some(&remote_record.endpoint),
    )
    .await
    .map_err(ShareError::MintFailed)?;

    let space_name = options
        .space_name
        .as_deref()
        .unwrap_or(DEFAULT_SPACE_NAME)
        .to_owned();
    let url = compose_launcher_url(&invite_outcome.url, &space_name, concept_name)?;

    Ok(ShareOutcome {
        url,
        remote_name: remote_record.name,
        remote_endpoint: remote_record.endpoint,
        concept_name: concept_name.to_owned(),
        space_name,
    })
}

/// Pick the remote whose endpoint gets embedded in the share
/// URL. Single-remote auto-selection mirrors `slide push`'s
/// "implicit when unambiguous" heuristic.
async fn resolve_remote(
    site: &SlideSite,
    explicit: Option<&str>,
) -> Result<RemoteRecord, ShareError> {
    if let Some(name) = explicit {
        return remote::find(site, name)
            .await?
            .ok_or_else(|| ShareError::UnknownRemote(name.to_owned()));
    }

    let mut remotes = remote::list(site).await?;
    match remotes.len() {
        0 => Err(ShareError::NoRemote),
        1 => Ok(remotes.remove(0)),
        _ => {
            let names = remotes
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Err(ShareError::AmbiguousRemote(names))
        }
    }
}

/// Append the launcher-specific `name=` and `then=` query
/// parameters to a base invite URL. The base URL already carries
/// `access=` (and optionally `remote=` plus a fragment-encoded
/// seed); `Url::query_pairs_mut` preserves both.
///
/// `then=` is the path *suffix* under `/space/<name>/`, not an
/// absolute path. Tonk-ui prefixes the recipient's actual local
/// name at navigation time so the URL works regardless of
/// whether the recipient renamed the space on the join form or
/// already had the subject mounted under a different name.
fn compose_launcher_url(
    base: &str,
    space_name: &str,
    concept_name: &str,
) -> Result<String, ShareError> {
    let mut url = Url::parse(base)
        .map_err(|e| ShareError::Io(format!("minted invite URL did not parse: {e}")))?;
    let then = format!(
        "branch/{branch}/concept/{concept_name}",
        branch = site::BRANCH_NAME,
    );
    url.query_pairs_mut()
        .append_pair("name", space_name)
        .append_pair("then", &then);
    Ok(url.into())
}

/// Crate-default UI base for the share launcher URL — same one
/// `slide invite` uses. Re-exported so tests and CLI formatters
/// don't need a direct dep on `tonk-invite`.
pub use tonk_invite::DEFAULT_BASE_URL as DEFAULT_UI_BASE;
