//! `tonk remote add` / `tonk remote list` / `tonk remote
//! set-upstream` — register and link UCAN-S3 access-service
//! remotes against the local site.
//!
//! The dialog primitives (`repository.remote(name).create(...)`,
//! `branch.set_upstream(&target)`) handle the wire-level wiring;
//! this module's value-add is mirroring the writes onto the
//! repo's meta branch as `Replica` / `Remote` / `TrackingBranch`
//! concepts so tonk-ui's existing read paths see tonk-created
//! remotes the same way they see worker-created ones.

use std::collections::HashMap;

use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::{Branch, SiteAddress, Upstream};
use dialog_varsig::Did;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tonk_schema::domain::remote as remote_dom;
use tonk_schema::{Remote as RemoteConcept, RemoteExecution, Replica};

use crate::ExitCode;
use crate::site::{self, TonkSite};

/// Name of the meta branch every tonk repo carries alongside
/// `main`. Matches the worker's `META_BRANCH` so tonk-ui sees
/// tonk's writes without configuration.
pub const META_BRANCH: &str = "meta";

/// Default remote name tonk auto-registers when joining an
/// invite that carries a `remote=` URL. Matches the worker's
/// `DEFAULT_REMOTE` so a single human-readable label
/// ("origin") flows across both surfaces.
pub const DEFAULT_REMOTE: &str = "origin";

/// One row of `tonk remote list` — also the shape `find`
/// returns and `tonk invite --remote` consumes when embedding
/// a remote URL in a freshly-minted invite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteRecord {
    /// Local name of the remote.
    pub name: String,
    /// Subject DID of the repository on the remote side. For an
    /// agent-shared site this matches the local repo's DID.
    pub subject: Did,
    /// UCAN access-service endpoint URL. Recovered by decoding
    /// the meta-branch `Address` claim back into a `SiteAddress`
    /// and pulling the URL off the inner `UcanAddress`.
    pub endpoint: String,
    /// Explicit immutable-artifact relay, absent on legacy records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revocation_url: Option<String>,
}

/// Outcome of [`add`].
#[derive(Debug)]
pub struct AddOutcome {
    /// Local name the remote was registered under.
    pub name: String,
    /// Subject DID — caller-supplied or defaulted to the local
    /// repo's DID.
    pub subject: Did,
    /// Endpoint URL, echoed for confirmation.
    pub endpoint: String,
    /// Explicit immutable-artifact relay, when configured.
    pub revocation_url: Option<String>,
}

/// Outcome of [`set_upstream`].
#[derive(Debug)]
pub struct UpstreamOutcome {
    /// Local branch whose upstream got rewritten (always
    /// [`crate::site::BRANCH_NAME`] for tonk today).
    pub local_branch: String,
    /// Remote name the upstream points at.
    pub remote: String,
    /// Branch on the remote the local branch now tracks.
    pub remote_branch: String,
}

/// Failure modes for the tonk remote API.
#[derive(Debug, Error)]
pub enum RemoteError {
    /// `set_upstream` was given a remote name that's not on the
    /// meta branch.
    #[error("remote '{0}' is not registered; add it with `tonk remote add` first")]
    UnknownRemote(String),
    /// Several remotes are registered and the caller named none, so
    /// there is no unambiguous choice to make on their behalf.
    #[error("several remotes are registered ({0}); name one with `--remote <NAME>`")]
    AmbiguousRemote(String),
    /// Anything else — dialog I/O, decoding, query failure.
    #[error("{0}")]
    Io(String),
}

impl RemoteError {
    /// CLI exit code for this failure mode.
    pub fn exit_code(&self) -> ExitCode {
        ExitCode::IoError
    }
}

/// Register a UCAN-S3 access-service remote against the local
/// site and write the meta-branch records the browser-side
/// worker also reads.
///
/// `subject_override` lets the caller point the remote at a
/// repo whose DID differs from the local one — tonk uses this
/// when joining an invite, where the remote tracks the
/// inviter's DID, not the joiner's.
pub async fn add(
    site: &TonkSite,
    name: &str,
    endpoint: &str,
    subject_override: Option<Did>,
) -> Result<AddOutcome, RemoteError> {
    add_with_revocation(site, name, endpoint, subject_override, None).await
}

/// Register a remote and its explicit invitation-revocation relay atomically.
pub async fn add_with_revocation(
    site: &TonkSite,
    name: &str,
    endpoint: &str,
    subject_override: Option<Did>,
    revocation_url: Option<&str>,
) -> Result<AddOutcome, RemoteError> {
    if let Some(revocation_url) = revocation_url {
        url::Url::parse(revocation_url)
            .map_err(|error| RemoteError::Io(format!("invalid revocation URL: {error}")))?;
    }
    let address = SiteAddress::from(UcanAddress::new(endpoint));

    // Dialog-side: provision the remote handle. This stamps a
    // RemoteAddress cell so subsequent push/pull can reach the
    // access service.
    let mut create = site.repository.remote(name).create(address.clone());
    let subject = match subject_override.clone() {
        Some(did) => {
            create = create.subject(did.clone());
            did
        }
        None => site.repository.did(),
    };
    create
        .perform(&site.operator)
        .await
        .map_err(|e| RemoteError::Io(format!("failed to create dialog remote: {e}")))?;

    // Meta-branch side: assert Replica + Remote so tonk-ui's
    // GET /api/repository/{name} surfaces this remote without
    // additional configuration.
    let meta = open_meta(site).await?;
    let replica = local_replica(site);
    let remote_concept = replica.remote(name, subject.clone(), &address);

    let mut transaction = meta
        .transaction()
        .assert(replica)
        .assert(remote_concept.clone());
    if let Some(revocation_url) = revocation_url {
        transaction = transaction.assert(RemoteExecution::new(&remote_concept, revocation_url));
    }
    transaction
        .commit()
        .perform(&site.operator)
        .await
        .map_err(|e| RemoteError::Io(format!("failed to write meta records: {e}")))?;

    Ok(AddOutcome {
        name: name.to_owned(),
        subject,
        endpoint: endpoint.to_owned(),
        revocation_url: revocation_url.map(str::to_owned),
    })
}

/// True when the local `main` branch already tracks an upstream.
/// `tonk remote add` consults this to decide whether the remote it
/// just registered should become the upstream by default — the
/// add-then-set-upstream pair is nearly always performed together,
/// and a first remote with no upstream wired is a foot-gun (writes
/// auto-sync only once an upstream exists).
pub async fn upstream_configured(site: &TonkSite) -> Result<bool, RemoteError> {
    let session = site
        .branch()
        .await
        .map_err(|e| RemoteError::Io(format!("failed to acquire branch: {e}")))?;
    Ok(session.handle().upstream().is_some())
}

/// Local name of the remote the site's `main` branch tracks, or
/// `None` when no upstream is wired.
///
/// The upstream cell records the remote under the same local name
/// it was registered with, so the answer compares straight against
/// a [`RemoteRecord`]'s `name`. `tonk invite` uses that comparison
/// to tell whether the remote it is about to embed in a link is the
/// one the repo actually pushes to. An upstream pointing at another
/// local branch names no remote and reads as `None` — tonk never
/// wires one, and there is no remote to compare against if it did.
pub async fn upstream_remote(site: &TonkSite) -> Result<Option<String>, RemoteError> {
    let session = site
        .branch()
        .await
        .map_err(|e| RemoteError::Io(format!("failed to acquire branch: {e}")))?;
    Ok(match session.handle().upstream() {
        Some(Upstream::Remote { remote, .. }) => Some(remote),
        Some(Upstream::Local { .. }) | None => None,
    })
}

/// Set the local `main` branch's upstream to `<remote>/main`,
/// writing the corresponding `TrackingBranch` and remote-side
/// `Branch` concepts on the meta branch.
pub async fn set_upstream(
    site: &TonkSite,
    remote_name: &str,
) -> Result<UpstreamOutcome, RemoteError> {
    // Find the remote's meta record so we have the address +
    // subject for the meta-side TrackingBranch write.
    let remote_record = find(site, remote_name)
        .await?
        .ok_or_else(|| RemoteError::UnknownRemote(remote_name.to_owned()))?;

    // Dialog side: load the remote, open its `main` branch,
    // wire the local `main` to track it.
    let remote_handle = site
        .repository
        .remote(remote_name)
        .load()
        .perform(&site.operator)
        .await
        .map_err(|e| RemoteError::Io(format!("failed to load remote '{remote_name}': {e}")))?;

    let upstream_branch = remote_handle
        .branch(site::BRANCH_NAME)
        .open()
        .perform(&site.operator)
        .await
        .map_err(|e| {
            RemoteError::Io(format!(
                "failed to open remote branch '{remote_name}/{branch}': {e}",
                branch = site::BRANCH_NAME,
            ))
        })?;

    let session = site
        .branch()
        .await
        .map_err(|e| RemoteError::Io(format!("failed to acquire branch: {e}")))?;
    session
        .handle()
        .set_upstream(&upstream_branch)
        .perform(&site.operator)
        .await
        .map_err(|e| RemoteError::Io(format!("failed to set upstream: {e}")))?;

    // Membership, roles, invitations, and replica metadata live on `meta`.
    // Track that branch beside `main` so a verified pull cannot observe
    // content without the signed relationship that authorizes this profile.
    let remote_meta = remote_handle
        .branch(META_BRANCH)
        .open()
        .perform(&site.operator)
        .await
        .map_err(|e| RemoteError::Io(format!("failed to open remote meta branch: {e}")))?;
    let local_meta = open_meta(site).await?;
    local_meta
        .set_upstream(&remote_meta)
        .perform(&site.operator)
        .await
        .map_err(|e| RemoteError::Io(format!("failed to set meta upstream: {e}")))?;

    // Meta side: rebuild the Remote concept (deterministic from
    // replica + name + subject + address) so we can hang a
    // remote-side Branch concept off it for the TrackingBranch
    // record.
    let meta = open_meta(site).await?;
    let replica = local_replica(site);
    let address = SiteAddress::from(UcanAddress::new(&remote_record.endpoint));
    let remote_concept = replica.remote(remote_name, remote_record.subject.clone(), &address);
    let tracked = remote_concept.branch(site::BRANCH_NAME);
    let tracking = replica.branch(site::BRANCH_NAME).set_upstream(&tracked);

    meta.transaction()
        .assert(tracked)
        .assert(tracking)
        .commit()
        .perform(&site.operator)
        .await
        .map_err(|e| RemoteError::Io(format!("failed to write tracking-branch records: {e}")))?;

    Ok(UpstreamOutcome {
        local_branch: site::BRANCH_NAME.to_owned(),
        remote: remote_name.to_owned(),
        remote_branch: site::BRANCH_NAME.to_owned(),
    })
}

/// Enumerate every remote registered on this replica.
///
/// Reads the meta branch's `Remote` concepts filtered by the
/// local replica entity, then decodes each `Address` claim back
/// into a `SiteAddress` so the endpoint URL is human-readable.
///
/// Rows tonk cannot act on are dropped silently: an address that
/// isn't a UCAN endpoint (legacy shapes, future variants), or a
/// subject that isn't a parseable DID. This is not just a display
/// filter — [`resolve`] counts what survives, so a repo with two
/// registered remotes where one fails to decode resolves the other
/// implicitly, as if it were the only one.
pub async fn list(site: &TonkSite) -> Result<Vec<RemoteRecord>, RemoteError> {
    use dialog_query::{Output as _, Query, Term};

    let meta = open_meta(site).await?;
    let replica_entity = local_replica(site).this().clone();

    let rows: Vec<RemoteConcept> = meta
        .query()
        .select(Query::<RemoteConcept> {
            this: Term::var("this"),
            name: Term::var("name"),
            origin: Term::from(remote_dom::Origin::from(replica_entity)),
            subject: Term::var("subject"),
            address: Term::var("address"),
        })
        .perform(&site.operator)
        .try_vec()
        .await
        .map_err(|e| RemoteError::Io(format!("remote enumeration failed: {e:?}")))?;
    let executions: Vec<RemoteExecution> = meta
        .query()
        .select(Query::<RemoteExecution> {
            this: Term::var("this"),
            revocation_url: Term::var("revocation_url"),
        })
        .perform(&site.operator)
        .try_vec()
        .await
        .map_err(|e| RemoteError::Io(format!("remote execution enumeration failed: {e:?}")))?;

    let mut out: Vec<RemoteRecord> = Vec::with_capacity(rows.len());
    for row in rows {
        let endpoint = match decode_endpoint(&row.address) {
            Some(url) => url,
            // Skip non-UCAN addresses — tonk only knows how to
            // talk to access-service endpoints, and surfacing
            // an entry the user can't push/pull through would
            // be misleading.
            None => continue,
        };
        let subject = match row.subject.0.to_string().parse::<Did>() {
            Ok(did) => did,
            Err(_) => continue,
        };
        out.push(RemoteRecord {
            name: row.name.0,
            subject,
            endpoint,
            revocation_url: executions
                .iter()
                .find(|execution| execution.this == row.this)
                .map(|execution| execution.revocation_url.0.clone()),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Look up one remote by name. Convenience wrapper around
/// [`list`], used by [`resolve`] and [`set_upstream`].
pub async fn find(site: &TonkSite, name: &str) -> Result<Option<RemoteRecord>, RemoteError> {
    let mut remotes: HashMap<String, RemoteRecord> = list(site)
        .await?
        .into_iter()
        .map(|r| (r.name.clone(), r))
        .collect();
    Ok(remotes.remove(name))
}

/// Pick the remote a command should act on.
///
/// `explicit` names one outright. Otherwise a lone registered remote is
/// the obvious choice — the same implicit-when-unambiguous reading
/// `tonk remote add` applies when it wires a first remote as the
/// upstream. No remotes at all means there is nothing to act on
/// (`None`, not an error — a local-only repo is a legitimate thing to
/// invite someone to), and several is a question only the caller can
/// answer.
///
/// "Lone" means lone *after* [`list`]'s filtering, which drops rows
/// tonk cannot push through. A repo holding one usable remote beside
/// one undecodable one resolves implicitly, with no mention of the
/// dropped row.
pub async fn resolve(
    site: &TonkSite,
    explicit: Option<&str>,
) -> Result<Option<RemoteRecord>, RemoteError> {
    if let Some(name) = explicit {
        let record = find(site, name)
            .await?
            .ok_or_else(|| RemoteError::UnknownRemote(name.to_owned()))?;
        return Ok(Some(record));
    }

    let mut remotes = list(site).await?;
    match remotes.len() {
        0 => Ok(None),
        1 => Ok(Some(remotes.remove(0))),
        _ => Err(RemoteError::AmbiguousRemote(
            remotes
                .iter()
                .map(|record| record.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        )),
    }
}

// ---------------------------------------------------------------- //
// Helpers                                                          //
// ---------------------------------------------------------------- //

/// Open (or create on first reference) the repo's meta branch.
/// Both `add` and `set_upstream` start by opening it; the
/// implicit-create-on-first-write semantics mean a fresh tonk
/// site doesn't need a separate meta-branch bootstrap step.
async fn open_meta(site: &TonkSite) -> Result<Branch, RemoteError> {
    site.repository
        .branch(META_BRANCH)
        .open()
        .perform(&site.operator)
        .await
        .map_err(|e| RemoteError::Io(format!("failed to open meta branch: {e}")))
}

/// Build the `Replica` concept for the local site. Deterministic
/// from `(profile DID, repo subject)` so callers can freely
/// reconstruct it without round-tripping through the meta branch.
/// The name is no longer part of the replica's identity — it lives
/// in the repository's own `tonk/repository` concept.
fn local_replica(site: &TonkSite) -> Replica {
    Replica::new(site.profile.did(), site.repository.did())
}

/// Decode a stored `tonk_schema::domain::remote::Address` back
/// into a UCAN endpoint URL, returning `None` for non-UCAN
/// site shapes.
fn decode_endpoint(address: &remote_dom::Address) -> Option<String> {
    let site = remote_dom::Address::decode(address).ok()?;
    match site {
        SiteAddress::Ucan(ucan) => Some(ucan.endpoint().to_owned()),
        _ => None,
    }
}
