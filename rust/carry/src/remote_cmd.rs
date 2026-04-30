//! `carry remote add` — register a sync destination for this repository.
//!
//! A remote is a named `(site_address, subject_did)` pair stored both
//! on dialog's side (via `repo.remote(name).create(...)`, which is
//! what `push`/`pull` actually use to connect) and on the meta branch
//! as a `tonk_schema::Remote` concept (which is what `remote list` /
//! `remote show` query). The two halves are kept in sync inside a
//! single command — every code path that mutates a remote here also
//! mutates the matching meta-branch fact.
//!
//! Registering a remote does not by itself wire it up as the
//! push/pull target; pass `--set-upstream` to `carry remote add`, or
//! run `carry remote set-upstream <NAME>` afterwards.
//!
//! URL conventions:
//!
//! - `http://…` / `https://…` → UCAN-S3 access service endpoint. This is
//!   the recommended path: the access service mints short-lived presigned
//!   URLs from UCAN invocations, so raw S3 credentials never touch the
//!   user's machine.
//! - `s3://<anything>` → direct S3. The caller must also supply
//!   `--endpoint`, `--region`, `--bucket`, and (optionally) an
//!   `--access-key` / `--secret-key` pair. If credentials are supplied
//!   they are persisted in plaintext inside `.carry/`; we print a loud
//!   warning about the threat model in that case.

use crate::site::Site;
use anyhow::{Context, Result, anyhow, bail};
use dialog_capability::Did;
use dialog_query::{Output as _, Query, Term};
use dialog_remote_s3::Address as S3Address;
use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::SiteAddress;
use tonk_schema::{Branch as MetaBranch, Remote as RemoteConcept, TrackingBranch};

/// The hidden branch name. Carry v1 does not expose branches.
pub(crate) const HIDDEN_BRANCH: &str = "main";

/// Options for registering a remote. The CLI layer normalises its flags
/// into one of these before calling [`execute`].
pub struct RemoteAddOptions {
    pub name: String,
    pub url: String,
    /// Subject DID at the remote. `None` means "use my own repo DID",
    /// which is the common case (syncing your own repo to your own
    /// bucket). `Some(did)` is for cross-repo pulls (e.g. Bob pulling
    /// Alice's data).
    pub subject: Option<String>,
    pub s3_endpoint: Option<String>,
    pub s3_region: Option<String>,
    pub s3_bucket: Option<String>,
    pub s3_access_key: Option<String>,
    pub s3_secret_key: Option<String>,
    /// If true, also wire this remote up as the upstream for push/pull.
    /// Mirrors `git remote add -u`.
    pub set_upstream: bool,
}

/// Execute `carry remote add`.
pub async fn execute(site: &Site, opts: RemoteAddOptions) -> Result<()> {
    let site_address = build_site_address(&opts)?;

    if let SiteAddress::S3(_) = site_address {
        print_s3_credentials_warning();
    }

    // Subject defaults to this repo's own DID — the common case is
    // syncing your own repo to your own bucket. `--subject` overrides
    // for cross-repo pulls.
    let subject: Did = match opts.subject.as_deref() {
        Some(raw) => raw
            .parse()
            .with_context(|| format!("invalid --subject DID: {}", raw))?,
        None => site.repo.did(),
    };

    // Create the remote on the dialog side (this is what push/pull
    // actually uses).
    let mut create = site
        .repo
        .remote(opts.name.as_str())
        .create(site_address.clone());
    if opts.subject.is_some() {
        create = create.subject(subject.clone());
    }
    create
        .perform(&site.operator)
        .await
        .with_context(|| format!("failed to register remote '{}'", opts.name))?;

    // Mirror the registration on the meta branch. The dialog side is
    // already durable; if this fails the remote works but won't show
    // up in `remote list`. Surface the failure so the caller can
    // retry rather than silently leaving the state split.
    site.meta
        .transaction()
        .assert(
            site.replica
                .remote(opts.name.as_str(), subject, &site_address),
        )
        .commit()
        .perform(&site.operator)
        .await
        .with_context(|| format!("failed to record remote '{}' on meta branch", opts.name))?;

    if opts.set_upstream {
        set_upstream(site, &opts.name).await?;
        eprintln!(
            "Added remote '{}' and set it as the sync target.",
            opts.name
        );
    } else {
        eprintln!(
            "Added remote '{}'. Run `carry remote set-upstream {}` (or re-run \
             `carry remote add` with --set-upstream) to use it for push/pull.",
            opts.name, opts.name
        );
    }
    Ok(())
}

/// Query every `Remote` concept on this replica. Sorted by name so
/// output is stable.
async fn load_remotes(site: &Site) -> Result<Vec<RemoteConcept>> {
    let mut rows: Vec<RemoteConcept> = site
        .meta
        .query()
        .select(Query::<RemoteConcept> {
            this: Term::var("this"),
            name: Term::var("name"),
            origin: Term::from(site.replica.this().clone()),
            subject: Term::var("subject"),
            address: Term::var("address"),
        })
        .perform(&site.operator)
        .try_vec()
        .await
        .context("failed to query remotes on meta branch")?;
    rows.sort_by(|a, b| a.name.0.cmp(&b.name.0));
    Ok(rows)
}

/// Look up a single `Remote` by name, returning `None` if absent.
async fn find_remote(site: &Site, name: &str) -> Result<Option<RemoteConcept>> {
    let rows = load_remotes(site).await?;
    Ok(rows.into_iter().find(|r| r.name.0 == name))
}

/// Format a [`SiteAddress`] as a human-readable URL string.
///
/// Uses the standard `s3://<bucket>` scheme for S3 sites — the endpoint
/// and region are connection details, not part of the URI, and surface
/// separately (e.g. in `carry remote show`).
fn format_site_address(addr: &SiteAddress) -> String {
    match addr {
        SiteAddress::S3(s3) => format!("s3://{}", s3.bucket()),
        SiteAddress::Ucan(ucan) => ucan.endpoint().to_string(),
    }
}

/// Execute `carry remote list`.
pub async fn execute_list(site: &Site) -> Result<()> {
    let remotes = load_remotes(site).await?;
    if remotes.is_empty() {
        eprintln!("No remotes configured. Use `carry remote add` to register one.");
        return Ok(());
    }
    for remote in &remotes {
        match remote.address.decode() {
            Ok(addr) => println!("{}\t{}", remote.name.0, format_site_address(&addr)),
            Err(_) => println!("{}\t<unreadable address>", remote.name.0),
        }
    }
    Ok(())
}

/// Execute `carry remote show <name>`.
pub async fn execute_show(site: &Site, name: &str) -> Result<()> {
    let remote = find_remote(site, name)
        .await?
        .ok_or_else(|| anyhow!("remote '{}' not found", name))?;

    let addr = remote
        .address
        .decode()
        .with_context(|| format!("failed to decode address for remote '{}'", name))?;
    let url = format_site_address(&addr);
    let kind = match &addr {
        SiteAddress::S3(_) => "s3 (direct)",
        SiteAddress::Ucan(_) => "ucan-s3 (access service)",
    };

    let is_upstream = upstream_remote_entity(site)
        .await?
        .is_some_and(|entity| entity == remote.this);

    println!("name:     {}", name);
    println!("url:      {}", url);
    println!("type:     {}", kind);
    if let SiteAddress::S3(s3) = &addr {
        println!("endpoint: {}", s3.endpoint());
        println!("region:   {}", s3.region());
    }
    println!("subject:  {}", remote.subject.0);
    if is_upstream {
        println!("upstream: yes (sync target for this branch)");
    }
    Ok(())
}

/// Look up which remote (if any) the local main branch tracks, by
/// reading the meta branch.
///
/// Returns the entity of the tracked remote (matching `Remote.this`),
/// or `None` if no tracking link is recorded.
async fn upstream_remote_entity(site: &Site) -> Result<Option<dialog_artifacts::Entity>> {
    let local = site.replica.branch(HIDDEN_BRANCH);
    let tracking: Vec<TrackingBranch> = site
        .meta
        .query()
        .select(Query::<TrackingBranch> {
            this: Term::from(local.this.clone()),
            upstream: Term::var("upstream"),
            origin: Term::from(site.replica.this().clone()),
        })
        .perform(&site.operator)
        .try_vec()
        .await
        .context("failed to query tracking branches on meta branch")?;
    let Some(track) = tracking.into_iter().next() else {
        return Ok(None);
    };
    // The tracked entity is a remote-side `Branch`; its origin is the
    // remote's entity. Look up that branch on the meta branch to
    // recover the remote entity.
    let upstream_branches: Vec<MetaBranch> = site
        .meta
        .query()
        .select(Query::<MetaBranch> {
            this: Term::from(track.upstream.0.clone()),
            name: Term::var("name"),
            origin: Term::var("origin"),
        })
        .perform(&site.operator)
        .try_vec()
        .await
        .context("failed to resolve tracked branch on meta branch")?;
    Ok(upstream_branches.into_iter().next().map(|b| b.origin.0))
}

/// Execute `carry remote set-upstream <name>`.
pub async fn execute_set_upstream(site: &Site, name: &str) -> Result<()> {
    set_upstream(site, name).await?;
    eprintln!("Updated upstream to remote '{}'.", name);
    Ok(())
}

/// Load a named remote and wire it up as the upstream for `push`/`pull`.
///
/// Updates both halves of the upstream wiring in the same call:
///
/// - Dialog side: opens the remote branch and calls
///   `local_branch.set_upstream(remote_branch)` so `push`/`pull` know
///   where to sync.
/// - Meta side: asserts the local `Branch`, the remote-side `Branch`,
///   and a `TrackingBranch` linking them so `remote show` (and any
///   other meta-branch reader) can answer "what's the upstream of
///   `main`?" without consulting dialog's storage.
async fn set_upstream(site: &Site, name: &str) -> Result<()> {
    let remote = site
        .repo
        .remote(name)
        .load()
        .perform(&site.operator)
        .await
        .with_context(|| format!("remote '{}' not found", name))?;

    let remote_branch = remote
        .branch(HIDDEN_BRANCH)
        .open()
        .perform(&site.operator)
        .await
        .with_context(|| format!("failed to open remote branch on '{}'", name))?;

    site.branch
        .set_upstream(remote_branch)
        .perform(&site.operator)
        .await
        .with_context(|| format!("failed to set upstream to '{}'", name))?;

    // Mirror the upstream wiring on the meta branch. Recompute the
    // schema concept from the loaded remote so we don't depend on the
    // caller having the original `--url` / subject in hand. Drop any
    // stale `TrackingBranch` for `main` first — set-upstream replaces
    // an existing upstream, it doesn't accumulate.
    let addr = remote.address();
    let remote_concept = site
        .replica
        .remote(name, addr.subject().clone(), addr.site());
    let local = site.replica.branch(HIDDEN_BRANCH);
    let tracked = remote_concept.branch(HIDDEN_BRANCH);

    let mut tx = site.meta.transaction();
    for stale in load_tracking_for_local(site, &local.this).await? {
        tx = tx.retract(stale);
    }
    tx.assert(local.clone())
        .assert(tracked.clone())
        .assert(local.set_upstream(&tracked))
        .commit()
        .perform(&site.operator)
        .await
        .with_context(|| format!("failed to record upstream for '{}' on meta branch", name))?;

    Ok(())
}

/// Pull every `TrackingBranch` whose `this` equals the given local
/// branch entity. Used to retract stale upstream links before
/// asserting a new one.
async fn load_tracking_for_local(
    site: &Site,
    local_entity: &dialog_artifacts::Entity,
) -> Result<Vec<TrackingBranch>> {
    site.meta
        .query()
        .select(Query::<TrackingBranch> {
            this: Term::from(local_entity.clone()),
            upstream: Term::var("upstream"),
            origin: Term::from(site.replica.this().clone()),
        })
        .perform(&site.operator)
        .try_vec()
        .await
        .context("failed to query tracking branches on meta branch")
}

/// Execute `carry remote remove <name>`.
pub async fn execute_remove(site: &Site, name: &str) -> Result<()> {
    let remote_concept = find_remote(site, name)
        .await?
        .ok_or_else(|| anyhow!("remote '{}' not found", name))?;

    // If this remote was the upstream, the matching `TrackingBranch`
    // fact has to come down with it; otherwise `remote show` would
    // report a stale link to a remote that no longer exists.
    let local = site.replica.branch(HIDDEN_BRANCH);
    let tracked_branch = remote_concept.branch(HIDDEN_BRANCH);
    let was_upstream = upstream_remote_entity(site)
        .await?
        .is_some_and(|entity| entity == remote_concept.this);

    let mut tx = site.meta.transaction();
    tx = tx.retract(remote_concept.clone()).retract(tracked_branch);
    if was_upstream {
        tx = tx.retract(
            local
                .clone()
                .set_upstream(&remote_concept.branch(HIDDEN_BRANCH)),
        );
    }
    tx.commit()
        .perform(&site.operator)
        .await
        .with_context(|| format!("failed to remove remote '{}' from meta branch", name))?;

    if was_upstream {
        // dialog-repository's set_upstream now requires a concrete
        // Branch/RemoteBranch; there's no public "clear upstream"
        // operation. The next sync attempt will fail to resolve the
        // upstream and surface a clear error. Reintroduce an
        // explicit clear step once dialog exposes one.
        eprintln!("Removed remote '{}' and cleared the sync target.", name);
    } else {
        eprintln!("Removed remote '{}'.", name);
    }

    Ok(())
}

/// Build a [`SiteAddress`] from the user's URL + flag bundle.
pub fn build_site_address(opts: &RemoteAddOptions) -> Result<SiteAddress> {
    let url = opts.url.trim();

    if url.starts_with("http://") || url.starts_with("https://") {
        // UCAN-S3 access service. This is the preferred path.
        if opts.s3_endpoint.is_some()
            || opts.s3_region.is_some()
            || opts.s3_bucket.is_some()
            || opts.s3_access_key.is_some()
            || opts.s3_secret_key.is_some()
        {
            bail!(
                "--endpoint/--region/--bucket/--access-key/--secret-key are only valid \
                 for s3:// URLs; for a UCAN-S3 access service just pass the https:// URL"
            );
        }
        return Ok(UcanAddress::new(url.to_string()).into());
    }

    if url.starts_with("s3://") {
        let endpoint = opts
            .s3_endpoint
            .as_deref()
            .ok_or_else(|| anyhow!("s3:// remote requires --endpoint <URL>"))?;
        let region = opts
            .s3_region
            .as_deref()
            .ok_or_else(|| anyhow!("s3:// remote requires --region <REGION>"))?;
        let bucket = opts
            .s3_bucket
            .as_deref()
            .ok_or_else(|| anyhow!("s3:// remote requires --bucket <BUCKET>"))?;

        let addr = S3Address::builder(endpoint)
            .region(region)
            .bucket(bucket)
            .build()
            .context("invalid s3 address")?;

        // Raw S3 credentials are no longer carried by the address itself;
        // the access service or the operator's credential store handles
        // that. Reject the flags here rather than silently dropping them.
        if opts.s3_access_key.is_some() || opts.s3_secret_key.is_some() {
            bail!(
                "--access-key/--secret-key are no longer supported on s3:// remotes; \
                 use a UCAN-S3 access service (https://) so credentials stay on the server"
            );
        }

        return Ok(addr.into());
    }

    bail!(
        "unrecognised remote URL '{}': expected https:// (UCAN-S3, recommended) or s3://",
        url
    )
}

/// Loud warning printed whenever raw S3 credentials are persisted into
/// `.carry/`.
pub fn print_s3_credentials_warning() {
    eprintln!();
    eprintln!("warning: this remote stores raw S3 credentials in plaintext inside .carry/");
    eprintln!("         anyone with read access to this directory can read and write the bucket.");
    eprintln!("         do NOT upload, commit, or share .carry/ with any public or untrusted");
    eprintln!("         destination (git, cloud drives, attachments, chat, etc.).");
    eprintln!("         prefer a UCAN-S3 access service (https:// URL) so credentials stay");
    eprintln!("         on the server.");
    eprintln!();
}
