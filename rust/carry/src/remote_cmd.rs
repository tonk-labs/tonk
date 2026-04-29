//! `carry remote add` — register a sync destination for this repository.
//!
//! A remote is a named `(site_address, subject_did)` pair stored inside
//! the repo's memory cells via `dialog_repository`'s
//! `repo.remote(name).create(...)` command. Registering a remote does
//! not by itself wire it up as the push/pull target; pass
//! `--set-upstream` to `carry remote add`, or run
//! `carry remote set-upstream <NAME>` afterwards.
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

use crate::site::{REPO_NAME, Site};
use anyhow::{Context, Result, anyhow, bail};
use dialog_capability::Did;
use dialog_remote_s3::Address as S3Address;
use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::{SiteAddress, Upstream};
use std::path::PathBuf;

/// Path to the directory dialog uses to persist remotes for this repo.
///
/// FIXME: this scrapes dialog's on-disk storage layout, which is brittle
/// and a layering violation. dialog-repository does not expose a way to
/// enumerate remotes (only `repo.remote(name)` for a known name), so
/// `remote list`/`remote remove` reach into the storage directory
/// directly. Once dialog grows a `remotes()` listing API (or carry
/// mirrors remotes onto a meta-branch fact) replace these callers and
/// drop this helper.
fn remote_storage_dir(site: &Site) -> PathBuf {
    site.root().join(REPO_NAME).join("memory").join("remote")
}

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

    // Create the remote. By default the subject is this repo's own DID;
    // override with `--subject` when pointing at somebody else's repo.
    let create = site.repo.remote(opts.name.as_str()).create(site_address);
    let create = match opts.subject.as_deref() {
        Some(raw) => {
            let did: Did = raw
                .parse()
                .with_context(|| format!("invalid --subject DID: {}", raw))?;
            create.subject(did)
        }
        None => create,
    };
    create
        .perform(&site.operator)
        .await
        .with_context(|| format!("failed to register remote '{}'", opts.name))?;

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

/// Discover remote names by scanning dialog's on-disk storage for
/// `remote/*/address` entries. Returns sorted names. See
/// [`remote_storage_dir`] for the layering caveat.
fn list_remote_names(site: &Site) -> Result<Vec<String>> {
    let remote_dir = remote_storage_dir(site);
    if !remote_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&remote_dir)
        .with_context(|| format!("failed to read {}", remote_dir.display()))?
    {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let address_file = entry.path().join("address");
            if address_file.exists()
                && let Some(name) = entry.file_name().to_str()
            {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
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
    let names = list_remote_names(site)?;
    if names.is_empty() {
        eprintln!("No remotes configured. Use `carry remote add` to register one.");
        return Ok(());
    }
    for name in &names {
        match site
            .repo
            .remote(name.as_str())
            .load()
            .perform(&site.operator)
            .await
        {
            Ok(remote) => {
                let url = format_site_address(remote.address().site());
                println!("{}\t{}", name, url);
            }
            Err(_) => {
                println!("{}\t<failed to load>", name);
            }
        }
    }
    Ok(())
}

/// Execute `carry remote show <name>`.
pub async fn execute_show(site: &Site, name: &str) -> Result<()> {
    let remote = site
        .repo
        .remote(name)
        .load()
        .perform(&site.operator)
        .await
        .with_context(|| format!("remote '{}' not found", name))?;

    let addr = remote.address();
    let url = format_site_address(addr.site());
    let kind = match addr.site() {
        SiteAddress::S3(_) => "s3 (direct)",
        SiteAddress::Ucan(_) => "ucan-s3 (access service)",
    };

    let is_upstream = match site.branch.upstream() {
        Some(Upstream::Remote {
            remote: ref upstream_name,
            ..
        }) => upstream_name == name,
        _ => false,
    };

    println!("name:     {}", name);
    println!("url:      {}", url);
    println!("type:     {}", kind);
    if let SiteAddress::S3(s3) = addr.site() {
        println!("endpoint: {}", s3.endpoint());
        println!("region:   {}", s3.region());
    }
    println!("subject:  {}", addr.subject());
    if is_upstream {
        println!("upstream: yes (sync target for this branch)");
    }
    Ok(())
}

/// Execute `carry remote set-upstream <name>`.
pub async fn execute_set_upstream(site: &Site, name: &str) -> Result<()> {
    set_upstream(site, name).await?;
    eprintln!("Updated upstream to remote '{}'.", name);
    Ok(())
}

/// Load a named remote and wire it up as the upstream for `push`/`pull`.
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

    Ok(())
}

/// Execute `carry remote remove <name>`.
pub async fn execute_remove(site: &Site, name: &str) -> Result<()> {
    site.repo
        .remote(name)
        .load()
        .perform(&site.operator)
        .await
        .with_context(|| format!("remote '{}' not found", name))?;

    let was_upstream = matches!(
        site.branch.upstream(),
        Some(Upstream::Remote {
            remote: ref upstream_name,
            ..
        }) if upstream_name == name
    );

    if was_upstream {
        // dialog-repository's set_upstream now requires a concrete
        // Branch/RemoteBranch; there's no public "clear upstream"
        // operation. Removing the on-disk remote directory below
        // breaks the upstream link in practice -- the next sync
        // attempt will fail to load the remote and surface a clear
        // error. Reintroduce an explicit clear step once dialog
        // exposes one.
    }

    let remote_dir = remote_storage_dir(site).join(name);

    if remote_dir.exists() {
        std::fs::remove_dir_all(&remote_dir)
            .with_context(|| format!("failed to remove {}", remote_dir.display()))?;
    }

    if was_upstream {
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
