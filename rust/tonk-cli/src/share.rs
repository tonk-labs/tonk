//! `tonk share <kind> <target>` — push the local repo to its
//! upstream, mint an audience-open invite that embeds the
//! upstream's URL, and return a launcher URL the human can paste
//! into a browser.
//!
//! Three flavours, sharing a common skeleton:
//!
//! - [`share_concept`] points at tonk-ui's auto-rendered concept
//!   route — `then=concept/<name>` (maps to `/space/<name>/concept/<source>`).
//! - [`share_view`] points at the iframe viewer route — the
//!   target resolves to an entity URI carrying a `text/html`
//!   claim, and `then=view/<entity>` (maps to `/space/<name>/view/<entity>`).
//! - [`share_display`] points at the `<tonk-display>` route —
//!   `then=<subject>?view=<view-name>` (maps to `/space/<name>/<subject>`
//!   via the `*subject` wildcard). `<view-name>` is the view's anchor name;
//!   the view carries its own `concept`.
//!
//! The launcher URL extends the standard invite URL with two
//! extra query parameters:
//!
//! - `name=<space-name>` — pre-fills the join form's "Local
//!   name" field. The human can rename before submitting.
//! - `then=<path-suffix>` — tells tonk-ui where to navigate
//!   after a successful claim, *relative to* the space's root.
//!   Tonk-ui prefixes `/space/<actual-name>/` using whatever
//!   local name the recipient ended up with (which can differ
//!   from `name=` — e.g. when the recipient already had the
//!   subject mounted under another name and lands in the
//!   already-member auto-claim path).
//!
//! `then=` degrades gracefully when tonk-ui doesn't yet honour
//! it: the join still completes, the human just lands on the
//! default post-claim page and navigates to the target by hand.
//!
//! Tonk does not invent a new write path for either kind —
//! every byte that lands on the branch came from `tonk eval` or
//! another existing tonk subcommand.

use dialog_artifacts::Entity;
use thiserror::Error;
use url::{Url, form_urlencoded};

use crate::ExitCode;
use crate::invite::{self, InviteError};
use crate::remote::{self, RemoteError, RemoteRecord};
use crate::schema;
use crate::site::{self, TonkSite};
use crate::sync::{self, SyncError};
use crate::views;

/// Default local-name suggestion encoded into the launcher URL's
/// `name=` parameter. The join form pre-fills with this; the
/// human can rename before submitting. Plain enough to not
/// collide with whatever the human had in mind, but
/// recognisable as a tonk-originated share.
pub const DEFAULT_SPACE_NAME: &str = "shared";

/// Per-call knobs for [`share_concept`]. All optional — the
/// defaults match the most common agent flow (one configured
/// remote, a "shared" space name, the workspace's standard UI
/// base).
#[derive(Debug, Default, Clone)]
pub struct ShareOptions {
    /// Override the URL prefix the invite is built against.
    /// Mirrors the same option on `tonk invite`. `None` falls
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

/// Outcome of [`share_display`].
#[derive(Debug)]
pub struct ShareDisplayOutcome {
    /// The launcher URL — `then=` resolves to the
    /// `<tonk-display>` route with `?view=…` (the view's anchor
    /// name), and `?concept=…` only in carousel mode.
    pub url: String,
    /// Local name of the remote whose endpoint got embedded.
    pub remote_name: String,
    /// Endpoint URL.
    pub remote_endpoint: String,
    /// Bookmark the subject was resolved through, when the caller
    /// passed a name rather than a raw entity URI. `None` when
    /// the caller passed `did:key:…` directly.
    pub subject_name: Option<String>,
    /// Entity URI the subject resolved to. Always populated, even
    /// when the URL carries the bookmark name verbatim (tonk-ui
    /// resolves the bookmark client-side).
    pub subject_entity: Entity,
    /// View anchor name forwarded as `?view=`, when supplied.
    pub view_name: Option<String>,
    /// Concept (name or URI) forwarded as `?concept=`, when
    /// supplied — carousel mode only; with `--view` the view
    /// declares its own concept. Mirrors what the caller passed.
    pub concept: Option<String>,
    /// `name=` value embedded into the URL.
    pub space_name: String,
}

/// Outcome of [`share_view`].
#[derive(Debug)]
pub struct ShareViewOutcome {
    /// The launcher URL — same shape as [`ShareOutcome::url`]
    /// but with a `view/<entity>` `then=` suffix.
    pub url: String,
    /// Local name of the remote whose endpoint got embedded.
    pub remote_name: String,
    /// Endpoint URL.
    pub remote_endpoint: String,
    /// Bookmark the target was resolved through, when the caller
    /// passed a name rather than a raw DID. `None` when the
    /// caller passed `did:key:…` directly.
    pub view_name: Option<String>,
    /// Entity URI baked into the launcher path.
    pub entity: Entity,
    /// `name=` value embedded into the URL.
    pub space_name: String,
}

/// Failure modes for [`share_concept`], [`share_view`], and
/// [`share_display`].
#[derive(Debug, Error)]
pub enum ShareError {
    /// `<name>` doesn't resolve to a concept on the local
    /// branch. Most often a typo; `tonk concepts` lists what's
    /// available.
    #[error(
        "concept '{name}' is not defined on this branch; \
         run `tonk concepts` to see what's available"
    )]
    ConceptNotFound {
        /// The name that didn't resolve.
        name: String,
    },
    /// A bookmark name passed to [`share_view`] didn't resolve
    /// to any entity. `tonk views` lists what's available.
    #[error(
        "view '{target}' is not bookmarked on this branch; \
         run `tonk views` to see what's available"
    )]
    ViewNotFound {
        /// The bookmark or entity URI that didn't resolve.
        target: String,
    },
    /// A bookmark name passed to [`share_display`] didn't resolve
    /// to any entity. Distinct from [`Self::ViewNotFound`] so the
    /// error message doesn't misdirect the agent to `tonk views`,
    /// which only lists `text/html`-bearing entities.
    #[error("subject '{target}' is not bookmarked on this branch")]
    SubjectNotFound {
        /// The bookmark or entity URI that didn't resolve.
        target: String,
    },
    /// The resolved entity exists but carries no `text/html`
    /// claim, so the host route would 404 on its body. Refuse
    /// before minting an unusable launcher URL.
    #[error(
        "entity {entity} has no `text/html` claim — the host \
         route would 404 on it. Assert a body via a `view!` \
         head (or any other path that lands a `text/html` claim) \
         and retry."
    )]
    NotAView {
        /// The entity that lacks a `text/html` claim.
        entity: String,
    },
    /// The target string passed to [`share_view`] couldn't be
    /// parsed as either an entity URI or a non-empty bookmark
    /// name. Most often a malformed `did:key:` paste.
    #[error("invalid view target '{target}': {reason}")]
    InvalidTarget {
        /// The supplied target string.
        target: String,
        /// Why we couldn't interpret it.
        reason: String,
    },
    /// No remote is registered. The share flow needs a remote so
    /// the joined site can pull from somewhere.
    #[error(
        "no remote is registered; add one with `tonk remote add <name> <url>` \
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
    #[error("remote '{0}' is not registered; run `tonk remote list` to see what's there")]
    UnknownRemote(String),
    /// The local branch has no upstream configured. Without one,
    /// `tonk push` would fail and the share would mint an
    /// invite the human can't actually pull from.
    #[error(
        "branch '{branch}' has no upstream configured; \
         run `tonk remote set-upstream <remote>` first"
    )]
    UpstreamNotConfigured {
        /// Branch missing an upstream — always `main` for tonk.
        branch: String,
    },
    /// `tonk push` failed. Most commonly non-fast-forward when
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
/// and produce a launcher URL pointing at the chromed concept view
/// (`/space/<name>/concept/<concept_name>`).
///
/// Pre-flight ordering matters: each step that can fail
/// independently runs before the side-effecting ones (push, mint)
/// so an early error doesn't leave the user with half-applied
/// state. Specifically:
///
/// 1. Verify the concept exists.
/// 2. Run the shared share prep (resolve remote → check
///    upstream → push).
/// 3. Mint the invite.
/// 4. Compose the launcher URL.
pub async fn share_concept(
    site: &TonkSite,
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

    let remote_record = prepare_share(site, options.remote.as_deref()).await?;
    let space_name = effective_space_name(options.space_name.as_deref());
    // `then=` is a suffix under `/space/<name>/`; the concept route is
    // `space/:space/concept/:source` so the suffix must start with
    // `concept/`. No branch segment — the `{branch}@` prefix in the
    // `{branch}@{name}` space segment defaults to `main` when omitted.
    let then = format!("concept/{concept_name}");
    let url = mint_and_compose(
        site,
        options.ui_base.as_deref(),
        &remote_record,
        &space_name,
        &then,
    )
    .await?;

    Ok(ShareOutcome {
        url,
        remote_name: remote_record.name,
        remote_endpoint: remote_record.endpoint,
        concept_name: concept_name.to_owned(),
        space_name,
    })
}

/// Push the local repo and mint a launcher URL that points at an
/// existing `text/html` body via the host route's iframe viewer.
///
/// `target` is either a bookmark name (resolved via
/// `dialog.meta/name` on the branch) or a `did:key:…` entity URI.
/// Either way the resolved entity must already carry at least
/// one `text/html` claim — otherwise the host route would 404 on
/// the embedded path and we'd hand the human a useless URL.
///
/// Steps parallel [`share_concept`]:
///
/// 1. Resolve the target to an entity + optional bookmark name.
/// 2. Verify the entity has a `text/html` claim.
/// 3. Run the shared share prep (resolve remote → check
///    upstream → push).
/// 4. Mint the invite.
/// 5. Compose the launcher URL.
pub async fn share_view(
    site: &TonkSite,
    target: &str,
    options: ShareOptions,
) -> Result<ShareViewOutcome, ShareError> {
    let (entity, view_name) =
        resolve_bookmark_or_uri(site, target, |t| ShareError::ViewNotFound { target: t }).await?;
    if !views::entity_has_text_html(site, &entity)
        .await
        .map_err(|e| ShareError::Io(format!("text/html lookup failed: {e}")))?
    {
        return Err(ShareError::NotAView {
            entity: entity.to_string(),
        });
    }

    let remote_record = prepare_share(site, options.remote.as_deref()).await?;
    let space_name = effective_space_name(options.space_name.as_deref());
    // `then=` is a suffix under `/space/<name>/`; the view route is
    // `space/:space/view/:entity` so the suffix must start with `view/`.
    let then = format!("view/{entity}");
    let url = mint_and_compose(
        site,
        options.ui_base.as_deref(),
        &remote_record,
        &space_name,
        &then,
    )
    .await?;

    Ok(ShareViewOutcome {
        url,
        remote_name: remote_record.name,
        remote_endpoint: remote_record.endpoint,
        view_name,
        entity,
        space_name,
    })
}

/// Push the local repo and mint a launcher URL that points at
/// the `<tonk-display>` route with the supplied view and concept
/// selectors baked into the query string.
///
/// Subject resolution is identical to [`share_view`] (bookmark
/// name or `did:key:…` URI). The concept argument can be a concept
/// name (validated against the local schema) or a URI (passed
/// through verbatim). The view argument is forwarded without
/// validation: `<tonk-display>` resolves the name to a view
/// entity at render time, and a name that doesn't resolve
/// surfaces as an error in the UI (not a generic fallback) — so
/// a typo isn't caught here, it fails on the recipient's screen.
/// Omitting `--view` entirely is the only thing that selects
/// carousel mode.
///
/// Steps parallel [`share_view`]:
///
/// 1. Resolve the subject to an entity + optional bookmark.
/// 2. Validate the concept when it looks like a name.
/// 3. Run the shared share prep.
/// 4. Mint the invite.
/// 5. Compose the launcher URL with the `?view=&concept=` suffix.
pub async fn share_display(
    site: &TonkSite,
    subject: &str,
    view_name: Option<&str>,
    concept: Option<&str>,
    options: ShareOptions,
) -> Result<ShareDisplayOutcome, ShareError> {
    let (entity, subject_name) =
        resolve_bookmark_or_uri(site, subject, |t| ShareError::SubjectNotFound { target: t })
            .await?;

    // Validate the concept when it's a bare identifier. URI-shaped
    // concepts (anything containing `:`) pass through — same
    // convention as `did:key:…` subjects.
    if let Some(name) = concept.filter(|m| !m.contains(':')) {
        let concepts = schema::list_concepts(site)
            .await
            .map_err(|e| ShareError::Io(format!("failed to list concepts: {e}")))?;
        if !concepts.iter().any(|c| c.name == name) {
            return Err(ShareError::ConceptNotFound {
                name: name.to_owned(),
            });
        }
    }

    let remote_record = prepare_share(site, options.remote.as_deref()).await?;
    let space_name = effective_space_name(options.space_name.as_deref());

    // Prefer the bookmark name in the URL when the caller passed
    // one — tonk-ui resolves it back through the same Name index.
    // Bookmarks survive entity-URI changes (re-asserting a view
    // body produces a new entity); the name does not.
    let subject_segment = subject_name.as_deref().unwrap_or(subject);
    let then = compose_display_then(subject_segment, view_name, concept);
    let url = mint_and_compose(
        site,
        options.ui_base.as_deref(),
        &remote_record,
        &space_name,
        &then,
    )
    .await?;

    Ok(ShareDisplayOutcome {
        url,
        remote_name: remote_record.name,
        remote_endpoint: remote_record.endpoint,
        subject_name,
        subject_entity: entity,
        view_name: view_name.map(str::to_owned),
        concept: concept.map(str::to_owned),
        space_name,
    })
}

/// Build the `then=` suffix for a display share: a path under the
/// recipient's space root with optional `view` / `concept` query
/// parameters appended. The `view`/`concept` values are
/// form-urlencoded so a stray `&` or `?` in a name doesn't corrupt
/// the inner query when tonk-ui pastes it onto its space-root path.
/// The `subject` is left verbatim as a path segment — `did:key:…`
/// URIs need their `:` intact and bookmark names don't carry query
/// delimiters.
///
/// The display route is `space/:space/*subject` — a wildcard that
/// captures the remainder after the space prefix, so the suffix is
/// just `{subject}` with no leading `display/` keyword segment.
fn compose_display_then(subject: &str, view: Option<&str>, concept: Option<&str>) -> String {
    let mut path = subject.to_owned();
    let mut pairs = form_urlencoded::Serializer::new(String::new());
    let mut has_any = false;
    if let Some(view) = view {
        pairs.append_pair("view", view);
        has_any = true;
    }
    if let Some(concept) = concept {
        pairs.append_pair("concept", concept);
        has_any = true;
    }
    if has_any {
        path.push('?');
        path.push_str(&pairs.finish());
    }
    path
}

/// Map a `<target>` argument into an `(entity, optional_name)`
/// pair. `did:key:…` strings are taken as URIs; everything else
/// is looked up as a `dialog.meta/name` bookmark. `missing` is
/// invoked when the bookmark doesn't resolve — callers choose
/// between [`ShareError::ViewNotFound`] and
/// [`ShareError::SubjectNotFound`] so the message matches the
/// verb the user typed.
async fn resolve_bookmark_or_uri(
    site: &TonkSite,
    target: &str,
    missing: fn(String) -> ShareError,
) -> Result<(Entity, Option<String>), ShareError> {
    if target.is_empty() {
        return Err(ShareError::InvalidTarget {
            target: target.to_owned(),
            reason: "target must not be empty".to_owned(),
        });
    }
    // `did:key:…` is the only URI shape the rest of tonk accepts
    // as an entity. Anything else routes through the bookmark
    // path so a typo like `did:keey:…` surfaces as "no such
    // bookmark" rather than a parser stack trace.
    if target.starts_with("did:") {
        let entity: Entity = target.parse().map_err(|e| ShareError::InvalidTarget {
            target: target.to_owned(),
            reason: format!("not a valid entity URI: {e:?}"),
        })?;
        return Ok((entity, None));
    }
    let entity = views::entity_for_name(site, target)
        .await
        .map_err(|e| ShareError::Io(format!("bookmark lookup failed: {e}")))?
        .ok_or_else(|| missing(target.to_owned()))?;
    Ok((entity, Some(target.to_owned())))
}

/// Steps every share flavour does after target validation:
/// resolve the remote, verify the local branch has an upstream,
/// and push. Returns the [`RemoteRecord`] whose endpoint should
/// be embedded as `remote=` in the launcher URL.
async fn prepare_share(
    site: &TonkSite,
    explicit_remote: Option<&str>,
) -> Result<RemoteRecord, ShareError> {
    let remote_record = resolve_remote(site, explicit_remote).await?;
    let session = site
        .branch()
        .await
        .map_err(|e| ShareError::Io(format!("acquire branch: {e}")))?;
    if session.handle().upstream().is_none() {
        return Err(ShareError::UpstreamNotConfigured {
            branch: site::BRANCH_NAME.to_owned(),
        });
    }
    sync::push(site).await.map_err(ShareError::PushFailed)?;
    Ok(remote_record)
}

/// Mint the invite and assemble the launcher URL. Both share
/// flavours funnel through this once they've pinned down
/// `then=<suffix>`.
async fn mint_and_compose(
    site: &TonkSite,
    ui_base: Option<&str>,
    remote: &RemoteRecord,
    space_name: &str,
    then: &str,
) -> Result<String, ShareError> {
    let invite_outcome = invite::mint(site, ui_base, Some(&remote.endpoint))
        .await
        .map_err(ShareError::MintFailed)?;
    compose_launcher_url(&invite_outcome.url, space_name, then)
}

fn effective_space_name(explicit: Option<&str>) -> String {
    explicit.unwrap_or(DEFAULT_SPACE_NAME).to_owned()
}

/// Pick the remote whose endpoint gets embedded in the share
/// URL. Single-remote auto-selection mirrors `tonk push`'s
/// "implicit when unambiguous" heuristic.
async fn resolve_remote(
    site: &TonkSite,
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
fn compose_launcher_url(base: &str, space_name: &str, then: &str) -> Result<String, ShareError> {
    let mut url = Url::parse(base)
        .map_err(|e| ShareError::Io(format!("minted invite URL did not parse: {e}")))?;
    url.query_pairs_mut()
        .append_pair("name", space_name)
        .append_pair("then", then);
    Ok(url.into())
}

/// Crate-default UI base for the share launcher URL — same one
/// `tonk invite` uses. Re-exported so tests and CLI formatters
/// don't need a direct dep on `tonk-invite`.
pub use tonk_invite::DEFAULT_BASE_URL as DEFAULT_UI_BASE;
