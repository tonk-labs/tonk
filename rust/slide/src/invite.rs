//! `slide invite` / `slide join` — UCAN-delegation-chain mint
//! and claim, on the same wire format `tonk-ui` already speaks
//! via the [`tonk_invite`] crate.
//!
//! `mint` builds an audience-open delegation from the local
//! repo's subject DID to a freshly generated ephemeral signer,
//! encodes it into a paste-able URL, and prints it. The
//! recipient runs `claim` (here, or via `tonk-ui`) which
//! redelegates from the ephemeral key onto the recipient's
//! profile DID and persists the resulting chain — so the
//! recipient's authority on the inviter's repo is materialised
//! locally, ready for `slide push` / `slide pull` once a remote
//! is configured.

use std::path::{Path, PathBuf};

use dialog_capability::Subject as CapSubject;
use dialog_credentials::{Credential, Ed25519Signer, Ed25519Verifier, key::KeyExport};
use dialog_effects::space::{Space, SpaceExt as _};
use dialog_ucan::UcanDelegation;
use dialog_varsig::{Did, Principal};
use thiserror::Error;
use tonk_invite::{Invite, InviteAudience};
use url::Url;

use crate::ExitCode;
use crate::remote::{self, DEFAULT_REMOTE};
use crate::site::{self, SITE_DIRNAME, SiteConfig, SlideSite};

/// Default base URL for minted invites. Mirrors
/// [`tonk_invite::DEFAULT_BASE_URL`] — exposed here so
/// integration tests can reach it without depending on
/// `tonk-invite` directly.
pub use tonk_invite::DEFAULT_BASE_URL;

/// Outcome of [`mint`].
#[derive(Debug)]
pub struct InviteOutcome {
    /// The minted invite URL — base58-encoded delegation
    /// chain in `?access=`, ephemeral seed in the fragment
    /// (audience-open form).
    pub url: String,
    /// The local repository's subject DID (the entity the
    /// invite grants access to).
    pub subject: Did,
    /// The ephemeral signer's DID — the chain's tail audience.
    /// Anyone with the URL fragment can redelegate from this
    /// signer to themselves.
    pub audience: Did,
}

/// Outcome of [`claim`].
#[derive(Debug)]
pub struct ClaimOutcome {
    /// Subject DID the invite granted access to. The new
    /// `.tonk/` site's repository targets this subject — slide
    /// holds a verifier-only credential, with mutating authority
    /// flowing through the persisted delegation chain.
    pub subject: Did,
    /// Sync remote URL the inviter attached, if any. When
    /// present, [`claim`] also auto-registered it under
    /// [`auto_configured_remote`](Self::auto_configured_remote).
    pub remote_url: Option<Url>,
    /// Local name of the auto-registered remote (always
    /// [`crate::remote::DEFAULT_REMOTE`] when set), or `None` if
    /// the invite carried no `remote=` URL. Slide's normal
    /// post-join `slide pull` resolves this remote implicitly
    /// because it's the only one configured.
    pub auto_configured_remote: Option<String>,
}

/// Failure modes for [`mint`] / [`claim`].
#[derive(Debug, Error)]
pub enum InviteError {
    /// The supplied invite URL didn't parse, or its embedded
    /// chain was malformed.
    #[error("invalid invite: {0}")]
    InvalidInvite(String),
    /// `claim` was asked to bootstrap a `.tonk/` directory in a
    /// parent that already has one. Slide is single-site per
    /// directory; the user must remove or relocate the existing
    /// site first.
    #[error("a .tonk/ site already exists at {0}; remove or rename it before joining")]
    SiteAlreadyExists(PathBuf),
    /// Anything else — key generation, delegation building,
    /// storage I/O. Surfaced verbatim.
    #[error("{0}")]
    Io(String),
}

impl InviteError {
    /// CLI exit code for this failure mode.
    pub fn exit_code(&self) -> ExitCode {
        ExitCode::IoError
    }
}

/// Mint an audience-open invite for the local site.
///
/// `base_url` overrides [`DEFAULT_BASE_URL`] for the URL prefix —
/// useful when minting against a local tonk-ui dev deployment.
/// `remote_url`, when supplied, is embedded as the invite's
/// `remote=` parameter so the claimer auto-configures the same
/// access service after redeeming.
pub async fn mint(
    site: &SlideSite,
    base_url: Option<&str>,
    remote_url: Option<&str>,
) -> Result<InviteOutcome, InviteError> {
    let (signer, seed) = generate_ephemeral().await?;
    let audience = signer.did();

    let delegation: UcanDelegation = site
        .profile
        .access()
        .claim(&site.repository)
        .delegate(audience.clone())
        .perform(&site.operator)
        .await
        .map_err(|e| InviteError::Io(format!("failed to build delegation: {e}")))?;

    let parsed_remote = match remote_url {
        Some(raw) => Some(
            Url::parse(raw)
                .map_err(|e| InviteError::Io(format!("invalid remote URL '{raw}': {e}")))?,
        ),
        None => None,
    };

    let invite = Invite::new(
        delegation.into_chain(),
        InviteAudience::Open { seed },
        parsed_remote,
    )
    .await
    .map_err(|e| InviteError::Io(format!("failed to assemble invite: {e}")))?;

    let url = invite
        .to_url(base_url.unwrap_or(DEFAULT_BASE_URL))
        .map_err(|e| InviteError::Io(format!("failed to serialize invite URL: {e}")))?;

    Ok(InviteOutcome {
        url,
        subject: site.repository.did(),
        audience,
    })
}

/// Claim an invite, bootstrapping a fresh `.tonk/` under
/// `parent` whose repository targets the invited subject DID.
///
/// Steps:
///
/// 1. Refuse if `.tonk/` already exists at `parent` — slide is
///    single-site per directory and the join would clobber an
///    existing site.
/// 2. Parse the URL via [`Invite::parse_url`]; reject malformed
///    invites before touching disk.
/// 3. Stand up the on-disk `.tonk/` and build a slide operator
///    rooted there, opening (or creating) the local profile.
/// 4. Claim the invite to the profile's DID and persist the
///    resulting chain so the operator can present it on
///    subsequent push/pull operations.
/// 5. Mint a verifier-only credential keyed to the invited
///    subject DID and create a local space at `name == "main"`,
///    matching the layout `slide init` produces (so all the
///    later read paths work uniformly across init- and
///    join-bootstrapped sites).
pub async fn claim(
    parent: &Path,
    invite_url: &str,
    config: SiteConfig,
) -> Result<ClaimOutcome, InviteError> {
    let parent = parent.canonicalize().map_err(|e| {
        InviteError::Io(format!("could not canonicalize {}: {e}", parent.display()))
    })?;
    let root = parent.join(SITE_DIRNAME);
    if root.exists() {
        return Err(InviteError::SiteAlreadyExists(root));
    }

    let invite = Invite::parse_url(invite_url)
        .await
        .map_err(|e| InviteError::InvalidInvite(e.to_string()))?;

    std::fs::create_dir_all(&root)
        .map_err(|e| InviteError::Io(format!("failed to create {}: {e}", root.display())))?;

    let (profile, operator) = site::build_profile_and_operator(&root, &config)
        .await
        .map_err(|e| InviteError::Io(e.to_string()))?;

    let claimed = invite
        .claim(&profile.did())
        .await
        .map_err(|e| InviteError::InvalidInvite(e.to_string()))?;

    let subject = claimed.subject().clone();
    let remote_url = claimed.remote_url.clone();

    profile
        .access()
        .save(UcanDelegation(claimed.chain))
        .perform(&operator)
        .await
        .map_err(|e| InviteError::Io(format!("failed to persist delegation chain: {e}")))?;

    // Provision the local space at `main` keyed to the invited
    // subject's verifier DID. The space credential is verifier-
    // only; mutating authority flows through the operator chain
    // we just persisted.
    let verifier: Ed25519Verifier = subject.to_string().parse().map_err(|e| {
        InviteError::InvalidInvite(format!(
            "invite subject is not a valid Ed25519 did:key: {e:?}"
        ))
    })?;
    let credential = Credential::from(verifier);

    CapSubject::from(profile.did())
        .attenuate(Space::new(site::REPO_NAME))
        .create(credential)
        .perform(&operator)
        .await
        .map_err(|e| InviteError::Io(format!("failed to provision local space: {e}")))?;

    // Open the main branch via the standard load path so the
    // joined site's structure matches `slide init`'s output.
    // Errors here would mean the create succeeded but the open
    // failed — surfaced as Io so the user sees the underlying
    // dialog message.
    let joined = SlideSite::open_with(&root, config)
        .await
        .map_err(|e| InviteError::Io(format!("failed to open joined site: {e}")))?;

    // Wire the embedded remote (if any) onto the freshly
    // bootstrapped site. Match the worker's `DEFAULT_REMOTE` so
    // a single human-readable label flows across both
    // surfaces; the remote's subject is the inviter's DID
    // (carried through on the claim chain), not the joiner's.
    let mut auto_configured_remote: Option<String> = None;
    if let Some(url) = &remote_url {
        remote::add(&joined, DEFAULT_REMOTE, url.as_str(), Some(subject.clone()))
            .await
            .map_err(|e| {
                InviteError::Io(format!("failed to auto-register remote from invite: {e}"))
            })?;
        remote::set_upstream(&joined, DEFAULT_REMOTE)
            .await
            .map_err(|e| InviteError::Io(format!("failed to wire upstream from invite: {e}")))?;
        auto_configured_remote = Some(DEFAULT_REMOTE.to_owned());
    }

    Ok(ClaimOutcome {
        subject,
        remote_url,
        auto_configured_remote,
    })
}

/// Generate an ephemeral Ed25519 signer with an extractable
/// seed. Mirrors [`tonk_worker`'s helper] — wasm's default
/// `Ed25519Signer::generate` produces a non-extractable
/// WebCrypto key whose seed can't be embedded in the invite
/// URL, so the wasm path opts in via [`ExtractableKey`]. Slide
/// is native-only today, so the cfg gate is dormant; keeping it
/// in place lets a future `slide-wasm` reuse this code path.
///
/// [`tonk_worker`'s helper]: ../../tonk-worker/src/router/create_invite.rs
/// [`ExtractableKey`]: dialog_credentials::key::ExtractableKey
async fn generate_ephemeral() -> Result<(Ed25519Signer, [u8; 32]), InviteError> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    let signer = {
        use dialog_credentials::key::ExtractableKey;
        <Ed25519Signer as ExtractableKey>::generate()
            .await
            .map_err(|e| InviteError::Io(format!("failed to generate ephemeral key: {e}")))?
    };
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    let signer = Ed25519Signer::generate()
        .await
        .map_err(|e| InviteError::Io(format!("failed to generate ephemeral key: {e}")))?;

    let exported = signer
        .export()
        .await
        .map_err(|e| InviteError::Io(format!("failed to export ephemeral key: {e}")))?;

    let seed: [u8; 32] = match exported {
        KeyExport::Extractable(bytes) => bytes.as_slice().try_into().map_err(|_| {
            InviteError::Io(format!(
                "ephemeral seed has unexpected length {}, want 32",
                bytes.len()
            ))
        })?,
        #[allow(unreachable_patterns)]
        other => {
            return Err(InviteError::Io(format!(
                "ephemeral key export returned an unexpected variant ({other:?}); \
                 expected KeyExport::Extractable so the seed can be embedded in the invite URL"
            )));
        }
    };

    Ok((signer, seed))
}
