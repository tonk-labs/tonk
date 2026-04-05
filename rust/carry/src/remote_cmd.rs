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
use dialog_remote_s3::{Address as S3Address, S3Credentials};
use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::SiteAddress;

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

    if let SiteAddress::S3(ref addr) = site_address
        && addr.credentials().is_some()
    {
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
    let remote = create
        .perform(&site.operator)
        .await
        .with_context(|| format!("failed to register remote '{}'", opts.name))?;

    // v1: exactly one (hidden) branch. Open its counterpart on the remote
    // and wire it up as the upstream so `carry push` / `carry pull` have
    // somewhere to go without enumeration.
    let remote_branch = remote
        .branch(HIDDEN_BRANCH)
        .open()
        .perform(&site.operator)
        .await
        .with_context(|| format!("failed to open remote branch on '{}'", opts.name))?;

    site.branch
        .set_upstream(remote_branch)
        .perform(&site.operator)
        .await
        .with_context(|| format!("failed to set upstream to '{}'", opts.name))?;

    eprintln!(
        "Added remote '{}' and set it as the sync target.",
        opts.name
    );
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

        let mut addr = S3Address::new(endpoint, region, bucket);

        match (&opts.s3_access_key, &opts.s3_secret_key) {
            (Some(key), Some(secret)) => {
                addr = addr.with_credentials(S3Credentials::new(key, secret));
            }
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
