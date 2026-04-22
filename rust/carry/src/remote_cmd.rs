//! `carry remote add` — register a sync destination for this repository.
//!
//! A remote is a named `(site_address, subject_did)` pair stored inside
//! the repo's memory cells via `dialog_repository`'s
//! `repo.remote(name).create(...)` command. Once registered, `carry push`
//! and `carry pull` can talk to it.
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
use dialog_remote_s3::Address as S3Address;
use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::{SiteAddress, UpstreamState};

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
}

/// Execute `carry remote add`.
pub async fn execute(site: &Site, opts: RemoteAddOptions) -> Result<()> {
    let site_address = build_site_address(&opts)?;

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

    set_upstream(site, &opts.name).await?;

    eprintln!(
        "Added remote '{}' and set it as the sync target.",
        opts.name
    );
    Ok(())
}

/// Discover remote names by scanning the `.carry/{repo_did}/memory/` directory
/// for `remote/*/address` entries. Returns sorted names.
fn list_remote_names(site: &Site) -> Result<Vec<String>> {
    let memory_dir = site.root().join(site.repo_did()).join("memory");
    let remote_dir = memory_dir.join("remote");
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
fn format_site_address(addr: &SiteAddress) -> String {
    match addr {
        SiteAddress::S3(s3) => format!("s3://{}/{}", s3.endpoint().as_str(), s3.bucket()),
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
        Some(UpstreamState::Remote {
            name: ref upstream_name,
            ..
        }) => upstream_name == name,
        _ => false,
    };

    println!("name:     {}", name);
    println!("url:      {}", url);
    println!("type:     {}", kind);
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
        Some(UpstreamState::Remote {
            name: ref upstream_name,
            ..
        }) if upstream_name == name
    );

    if was_upstream {
        // dialog-repository doesn't have clear_upstream() yet, so we
        // point at a non-existent local branch -- the same pattern
        // dialog's own integration tests use to simulate "no remote".
        site.branch
            .set_upstream(UpstreamState::Local {
                branch: "nowhere".into(),
                tree: Default::default(),
            })
            .perform(&site.operator)
            .await
            .context("failed to clear upstream")?;
    }

    let remote_dir = site
        .root()
        .join(site.repo_did())
        .join("memory")
        .join("remote")
        .join(name);

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
            .with_context(|| format!("invalid s3:// address for {}", endpoint))?;

        match (&opts.s3_access_key, &opts.s3_secret_key) {
            (Some(_), Some(_)) => bail!(
                "direct s3:// remotes with --access-key/--secret-key are not yet \
                 supported; use a https:// UCAN-S3 access service URL instead"
            ),
            (None, None) => {}
            _ => bail!("--access-key and --secret-key must be supplied together"),
        }

        return Ok(addr.into());
    }

    bail!(
        "unrecognised remote URL '{}': expected https:// (UCAN-S3, recommended) or s3://",
        url
    )
}
