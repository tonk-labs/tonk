//! `tonk invite` / `tonk join` — UCAN-delegation-chain mint
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
//! locally, ready for `tonk push` / `tonk pull` once a remote
//! is configured.

use dialog_capability::Subject;
use dialog_effects::Use;
use std::path::{Path, PathBuf};

use dialog_credentials::{Ed25519Signer, key::KeyExport};
use dialog_ucan::UcanDelegation;
use dialog_varsig::{Did, Principal};
use thiserror::Error;
use tonk_invite::shortcut::{ShortcutRequest, is_shortcut, resolve_location};
use tonk_invite::{Invite, InviteAudience};
use tonk_schema::{Invitation, InvitationExecution, InvitedVia, MemberRole, Membership};
use url::Url;

use crate::ExitCode;
use crate::remote::{self, DEFAULT_REMOTE, META_BRANCH};
use crate::site::{self, SiteConfig, TonkSite};
use crate::staged_directory::StagedDirectory;
use crate::sync;

/// Default base URL for minted invites. Mirrors
/// [`tonk_invite::DEFAULT_BASE_URL`] — exposed here so
/// integration tests can reach it without depending on
/// `tonk-invite` directly.
pub use tonk_invite::DEFAULT_BASE_URL;

/// Environment variable that opts a mint out of shortening, mirroring
/// the `--no-shorten` flag.
///
/// Shortening is a live `PUT` to the link's own origin. When no remote
/// resolves, that origin is [`DEFAULT_BASE_URL`]'s — production — so
/// without a way off this path any test of the no-remote base arm
/// writes to the real shortcut store.
pub const NO_SHORTEN_ENV: &str = "TONK_NO_SHORTEN";

/// Whether a mint should shorten its link.
///
/// Off when `--no-shorten` was passed (`no_shorten_flag`) or when
/// [`NO_SHORTEN_ENV`] is set to a value other than empty / `0` /
/// `false` / `no`.
pub fn shorten_enabled(no_shorten_flag: bool) -> bool {
    !no_shorten_flag
        && !crate::auto_sync::env_value_opts_out(std::env::var(NO_SHORTEN_ENV).ok().as_deref())
}

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
    /// `.tonk/` site's repository targets this subject — tonk
    /// holds a verifier-only credential, with mutating authority
    /// flowing through the persisted delegation chain.
    pub subject: Did,
    /// Sync remote URL the inviter attached, if any. When
    /// present, [`claim`] also auto-registered it under
    /// [`auto_configured_remote`](Self::auto_configured_remote).
    pub remote_url: Option<Url>,
    /// Local name of the auto-registered remote (always
    /// [`crate::remote::DEFAULT_REMOTE`] when set), or `None` if
    /// the invite carried no `remote=` URL. When set, [`claim`]
    /// also performs an initial pull from it (see [`synced`]).
    ///
    /// [`synced`]: Self::synced
    pub auto_configured_remote: Option<String>,
    /// Whether the initial post-join pull from the auto-configured
    /// remote succeeded and brought upstream state into the fresh
    /// local `main`. `false` when the invite carried no remote, or
    /// when the pull failed (e.g. the endpoint was unreachable) —
    /// join still succeeds, and the user can retry with `tonk pull`.
    pub synced: bool,
}

/// Failure modes for [`mint`] / [`claim`].
#[derive(Debug, Error)]
pub enum InviteError {
    /// The supplied invite URL didn't parse, or its embedded
    /// chain was malformed.
    #[error("invalid invite: {0}")]
    InvalidInvite(String),
    /// `claim` was asked to bootstrap a site directory that
    /// already exists. The join must never clobber existing site
    /// storage; the user removes it (or picks another space name)
    /// first.
    ///
    /// The likeliest way to reach this without having noticed is
    /// `tonk space rm --keep-data` under the same name, which leaves
    /// data here that no registry entry mentions — so the message
    /// names both ways out rather than just "remove it".
    #[error(
        "a site already exists at {0}\n\
         it belongs to no registered space; adopt it with \
         `tonk space new <name> --site {0}`,\n\
         delete the directory, or join under another space name"
    )]
    SiteAlreadyExists(PathBuf),
    /// Anything else — key generation, delegation building,
    /// storage I/O. Surfaced verbatim.
    #[error("{0}")]
    Io(String),
}

impl crate::Coded for InviteError {
    /// CLI exit code for this failure mode.
    fn exit_code(&self) -> ExitCode {
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
    site: &TonkSite,
    base_url: Option<&str>,
    remote_url: Option<&str>,
) -> Result<InviteOutcome, InviteError> {
    mint_for(site, base_url, remote_url, None, None).await
}

/// Mint an audience-open invite with an explicit revocation relay.
pub async fn mint_with_relay(
    site: &TonkSite,
    base_url: Option<&str>,
    remote_url: Option<&str>,
    revocation_url: Option<&str>,
) -> Result<InviteOutcome, InviteError> {
    mint_for(site, base_url, remote_url, revocation_url, None).await
}

/// Mint a seed-free invite targeted to an exact recipient root DID.
pub async fn mint_targeted(
    site: &TonkSite,
    base_url: Option<&str>,
    remote_url: Option<&str>,
    recipient_root: &str,
) -> Result<InviteOutcome, InviteError> {
    mint_for(site, base_url, remote_url, None, Some(recipient_root)).await
}

/// Mint a root-targeted invite with an explicit revocation relay.
pub async fn mint_targeted_with_relay(
    site: &TonkSite,
    base_url: Option<&str>,
    remote_url: Option<&str>,
    revocation_url: Option<&str>,
    recipient_root: &str,
) -> Result<InviteOutcome, InviteError> {
    mint_for(
        site,
        base_url,
        remote_url,
        revocation_url,
        Some(recipient_root),
    )
    .await
}

async fn mint_for(
    site: &TonkSite,
    base_url: Option<&str>,
    remote_url: Option<&str>,
    revocation_url: Option<&str>,
    recipient_root: Option<&str>,
) -> Result<InviteOutcome, InviteError> {
    // Push local state to the upstream before minting, so a joiner
    // receives current repo state — including the stdlib seed that
    // `tonk space new` committed before any upstream existed. No-op
    // when the branch has no upstream (a local-only invite).
    // Pull-before-push reconciles a possibly advanced upstream,
    // best-effort; the push error is authoritative.
    let has_upstream = {
        let session = site
            .branch()
            .await
            .map_err(|e| InviteError::Io(format!("acquire branch: {e}")))?;
        session.handle().upstream().is_some()
    };
    if has_upstream {
        if let Err(e) = sync::pull(site).await {
            eprintln!("warning: pull before invite failed: {e}");
        }
        sync::push(site)
            .await
            .map_err(|e| InviteError::Io(format!("push before invite failed: {e}")))?;
    }

    let (audience, invite_audience) = match recipient_root {
        Some(root) => (
            root.parse()
                .map_err(|error| InviteError::Io(format!("invalid recipient root DID: {error}")))?,
            InviteAudience::Scoped,
        ),
        None => {
            let (signer, seed) = generate_ephemeral().await?;
            (signer.did(), InviteAudience::Open { seed })
        }
    };

    let parsed_remote = match remote_url {
        Some(raw) => Some(
            Url::parse(raw)
                .map_err(|e| InviteError::Io(format!("invalid remote URL '{raw}': {e}")))?,
        ),
        None => None,
    };

    // With a remote, the leaf is signed with the endpoint in its
    // `home.address` meta so the grant and the address travel together.
    // A local-only invite has no endpoint to name and delegates plainly.
    let mut delegate = site
        .profile
        .access()
        .claim(Subject::from(site.repository.did()).attenuate(Use))
        .delegate(audience.clone());
    if let Some(remote) = &parsed_remote {
        delegate = delegate.meta(tonk_invite::home_address_meta(remote));
    }
    let delegation: UcanDelegation = delegate
        .perform(&site.operator)
        .await
        .map_err(|e| InviteError::Io(format!("failed to build delegation: {e}")))?;
    let chain = delegation.into_chain();

    // A remote with no relay configured is no longer a reason to refuse: a
    // revocation is an ordinary `ucan/revoke` invocation, so it goes to the
    // access service like everything else, and a separate relay is something
    // a deployment may still name but no longer has to.
    let relay = match revocation_url {
        Some(raw) => Some(
            Url::parse(raw)
                .map_err(|error| InviteError::Io(format!("invalid revocation URL: {error}")))?,
        ),
        None => None,
    };
    let invite = Invite::new(chain, invite_audience, parsed_remote)
        .await
        .map_err(|e| InviteError::Io(format!("failed to assemble invite: {e}")))?
        .with_revocation_url(relay);

    let url = invite
        .to_url(base_url.unwrap_or(DEFAULT_BASE_URL))
        .map_err(|e| InviteError::Io(format!("failed to serialize invite URL: {e}")))?;

    // Record the invitation on the repo's meta branch — the durable,
    // secret-free half of the invite (the seed stays in the URL).
    let invitation = Invitation::from_chain(&invite.chain)
        .expect("Invite invariant: chain has a specific subject");
    let execution = InvitationExecution::new(
        &invitation,
        if matches!(&invite.audience, InviteAudience::Open { .. }) {
            "open"
        } else {
            "scoped"
        },
    );
    let meta = site
        .repository
        .branch(META_BRANCH)
        .open()
        .perform(&site.operator)
        .await
        .map_err(|e| InviteError::Io(format!("failed to open meta branch: {e}")))?;
    meta.transaction()
        .assert(invitation)
        .assert(execution)
        .commit()
        .perform(&site.operator)
        .await
        .map_err(|e| InviteError::Io(format!("failed to record invitation: {e}")))?;

    Ok(InviteOutcome {
        url,
        subject: site.repository.did(),
        audience,
    })
}

/// Derive the invite base URL from a remote's endpoint.
///
/// The invite has to live on the remote's own origin. That origin is
/// the deployment actually serving the repo, and — because the
/// shortcut service is same-origin by construction — the only one
/// whose `PUT /@` can answer. This is the CLI's stand-in for the
/// worker's `location.origin`, which the browser mint path reads
/// straight off its own scope.
///
/// Any userinfo on the endpoint is stripped. A registered remote URL
/// carrying credentials would otherwise ride them into a link printed
/// to stdout and pasted to whoever is being invited.
///
/// # Errors
///
/// Returns an error if `endpoint` doesn't parse, or has no origin to
/// hang `/join` off (a `data:` or `mailto:` URL, say).
pub fn base_url_for_remote(endpoint: &str) -> Result<String, InviteError> {
    let mut parsed = Url::parse(endpoint).map_err(|e| {
        InviteError::Io(format!(
            "remote endpoint '{endpoint}' is not a valid URL: {e}"
        ))
    })?;
    // Both setters fail only on a URL that cannot have credentials
    // (`data:`, `mailto:`) — which has no usable origin either, so the
    // join below reports it. Nothing to add here.
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    parsed.join("/join").map(String::from).map_err(|e| {
        InviteError::Io(format!(
            "remote endpoint '{endpoint}' has no usable origin: {e}"
        ))
    })
}

/// Claim an invite, bootstrapping a fresh site at `root` (the
/// site directory itself — the caller picks it, typically the
/// canonical `spaces/<name>/` dir) whose repository targets the
/// invited subject DID.
///
/// Steps:
///
/// 1. Refuse if `root` already exists — the join must never
///    clobber existing site storage.
/// 2. Parse the URL via [`Invite::parse_url`]; reject malformed
///    invites before touching disk.
/// 3. Stand up a hidden sibling directory and build a tonk operator rooted
///    there, opening (or creating) the local profile.
/// 4. Claim the invite to the profile's DID and persist the
///    resulting chain so the operator can present it on
///    subsequent push/pull operations.
/// 5. Mint a verifier-only credential keyed to the invited
///    subject DID and create a local space at `name == "main"`,
///    matching the layout `tonk space new` produces (so all the
///    later read paths work uniformly across space-new- and
///    join-bootstrapped sites).
/// 6. If the invite carried a `remote=` URL, register it, set it
///    as the local `main`'s upstream, and pull once — a clean
///    fast-forward into the freshly-created branch, so the joiner
///    starts from the upstream's state rather than an empty branch
///    that would diverge on the first local write.
/// 7. Drop repository handles and publish the completed sibling at `root` with
///    one rename.
pub async fn claim(
    root: &Path,
    invite_url: &str,
    config: SiteConfig,
) -> Result<ClaimOutcome, InviteError> {
    if root.exists() {
        return Err(InviteError::SiteAlreadyExists(root.to_path_buf()));
    }

    // Short links (`/@/{hash}#seed`) resolve to the long form first —
    // the browser gets this from the 301 + fragment inheritance; here
    // it's done by hand.
    let resolved;
    let invite_url = if is_shortcut(invite_url) {
        resolved = resolve_shortcut(invite_url).await?;
        resolved.as_str()
    } else {
        invite_url
    };

    let invite = Invite::parse_url(invite_url)
        .await
        .map_err(|e| InviteError::InvalidInvite(e.to_string()))?;
    let invitation = Invitation::from_chain(&invite.chain)
        .expect("Invite invariant: chain has a specific subject");
    let invitation_execution = InvitationExecution::new(
        &invitation,
        if matches!(&invite.audience, InviteAudience::Open { .. }) {
            "open"
        } else {
            "scoped"
        },
    );

    let stage = StagedDirectory::beside(root, "join").map_err(|error| {
        InviteError::Io(format!(
            "failed to stage joined site for {}: {error:#}",
            root.display()
        ))
    })?;
    let staged_root = stage.path().canonicalize().map_err(|error| {
        InviteError::Io(format!(
            "could not canonicalize join stage {} for destination {}: {error}",
            stage.path().display(),
            root.display()
        ))
    })?;

    let (profile, operator) = site::build_profile_and_operator(&staged_root, &config)
        .await
        .map_err(|e| InviteError::Io(e.to_string()))?;

    // An invite already carries the authority needed to join. A linked
    // profile claims to its durable account root; before linking, the
    // device's account is the ONBOARDING account — a real account
    // custodied locally — so the join is durable to the same identity
    // creates delegate to, and the sign-in rotation carries it forward.
    let member = match crate::identity::local_root_with_operator(&profile, &operator)
        .await
        .map_err(|e| InviteError::Io(e.to_string()))?
    {
        Some(root) => root
            .root_did
            .parse()
            .map_err(|e| InviteError::Io(format!("stored root DID is invalid: {e}")))?,
        None => {
            use dialog_varsig::Principal as _;
            let store_operator = crate::account_state::store_operator_with_config(
                &profile,
                &config.account_store,
                &config.profile_name,
                config.profile_directory.clone(),
            )
            .await
            .map_err(|e| InviteError::Io(format!("{e:#}")))?;
            let secret = crate::onboarding::account(&profile, &store_operator)
                .await
                .map_err(|e| InviteError::Io(format!("{e:#}")))?;
            secret
                .signer()
                .await
                .map_err(|e| InviteError::Io(format!("the onboarding signer did not derive: {e}")))?
                .did()
        }
    };
    let claimed = invite
        .claim(&member)
        .await
        .map_err(|e| InviteError::InvalidInvite(e.to_string()))?;

    let subject = claimed.subject().clone();
    let remote_url = claimed.remote_url.clone();
    let revocation_url = claimed.revocation_url.clone();

    // Install only the reusable root-ending authority and verifier-backed
    // repository. Invite-specific roster/provenance writes remain below.
    let chain = claimed.chain.clone();
    let joined = site::mount_delegated_with(&staged_root, profile, operator, claimed.chain, config)
        .await
        .map_err(|e| InviteError::Io(format!("failed to mount joined site: {e:#}")))?;
    retain_claim_authority(&joined, chain).await;

    // Wire the embedded remote (if any) onto the freshly
    // bootstrapped site. Match the worker's `DEFAULT_REMOTE` so
    // a single human-readable label flows across both
    // surfaces; the remote's subject is the inviter's DID
    // (carried through on the claim chain), not the joiner's.
    let mut auto_configured_remote: Option<String> = None;
    let mut synced = false;
    if let Some(url) = &remote_url {
        remote::add_with_revocation(
            &joined,
            DEFAULT_REMOTE,
            url.as_str(),
            Some(subject.clone()),
            revocation_url.as_ref().map(Url::as_str),
        )
        .await
        .map_err(|e| InviteError::Io(format!("failed to auto-register remote from invite: {e}")))?;
        remote::set_upstream(&joined, DEFAULT_REMOTE)
            .await
            .map_err(|e| InviteError::Io(format!("failed to wire upstream from invite: {e}")))?;
        auto_configured_remote = Some(DEFAULT_REMOTE.to_owned());

        // Pull upstream state into the just-created local `main`. The join
        // provisioned `main` fresh and hasn't written to it, so this is a
        // clean fast-forward — the joiner ends up with a faithful copy of the
        // upstream, not an empty branch that silently diverges the moment they
        // (or their agent) start asserting. Best-effort: an unreachable remote
        // shouldn't fail an otherwise-complete join — the user can retry with
        // `tonk pull` — but a real sync error is worth surfacing.
        match sync::pull(&joined).await {
            Ok(_) => synced = true,
            Err(e) => eprintln!(
                "warning: joined, but the initial pull from '{DEFAULT_REMOTE}' failed: {e}\n\
                 run `tonk pull` before making changes so you don't diverge from upstream"
            ),
        }
    }

    record_claim_roster(&joined, &member, &subject, invitation, invitation_execution).await?;

    // A roster row that never leaves this device converges with nobody, so
    // the join is only finished once it is published. Best-effort for the
    // same reason the pull is: an unreachable remote must not fail a join
    // that otherwise completed, and the next `tonk push` carries the row.
    // Only when the pull succeeded — pushing onto an upstream this replica
    // never reconciled with is how a joiner diverges.
    if synced && let Err(e) = sync::push(&joined).await {
        eprintln!(
            "warning: joined, but publishing this device's roster row failed: {e}\n\
             run `tonk push` so the space's other members can see you"
        );
    }

    joined.reactor.shutdown();
    drop(joined);
    stage.publish().map_err(|error| {
        InviteError::Io(format!(
            "joined site was completed in a stage, but publication at {root} did not settle cleanly: {error:#}; if {root} is absent, retry the invite; if it is present, never overwrite or delete it merely to retry—verify its repository subject and adopt it only if it is the expected joined site with `tonk space new <available-name> --site {root}`",
            root = root.display()
        ))
    })?;

    Ok(ClaimOutcome {
        subject,
        remote_url,
        auto_configured_remote,
        synced,
    })
}

/// Record the claim on the joined space's content branch: the invitation it
/// came through, this member's roster row, and the provenance stamp.
///
/// The content branch, not `meta`. Only upstreamed branches sync, so a
/// membership written to `meta` never reaches the space's owner or its other
/// members — and, since the roster is now what names a space's owner and this
/// device's role in it, would leave `tonk space` showing a space this
/// device legitimately joined as one whose roster holds no row of ours. The
/// worker's claim path writes the same facts to the same branch.
///
/// Runs after the initial pull, never before: the join provisions `main`
/// deliberately empty so that pull is a clean fast-forward, and a row
/// committed ahead of it would make the joiner's first sync a merge. The
/// caller publishes the commit afterwards.
///
/// Both stamps are first-wins, mirroring the worker. The role is the one that
/// matters: `MemberRole` is cardinality-one on the membership entity, so
/// asserting `member` over a row the pull just brought down would demote
/// Retain the claimed chain into the space's content branch, so the hop
/// that admits this member is provable from the space itself and an admin
/// can revoke it without touching the invite everyone else used. Best
/// effort: the join is complete once the authority is saved locally.
async fn retain_claim_authority(joined: &TonkSite, chain: dialog_ucan_core::DelegationChain) {
    let session = match joined.branch().await {
        Ok(session) => session,
        Err(error) => {
            eprintln!("warning: claimed chain not retained on the space: {error}");
            return;
        }
    };
    if let Err(error) = session
        .handle()
        .delegations()
        .retain(UcanDelegation(chain))
        .perform(&joined.operator)
        .await
    {
        eprintln!("warning: claimed chain not retained on the space: {error}");
    }
}

/// whoever it names — including a founder claiming an invite to their own
/// space, whose profile is shared with every other site on this machine.
async fn record_claim_roster(
    joined: &TonkSite,
    member: &Did,
    subject: &Did,
    invitation: Invitation,
    invitation_execution: InvitationExecution,
) -> Result<(), InviteError> {
    use dialog_query::{Output as _, Query, Term};
    use tonk_schema::prelude::DidExt as _;

    let membership = Membership::new(member.clone(), subject.clone());
    let session = joined
        .branch()
        .await
        .map_err(|e| InviteError::Io(format!("failed to open the roster branch: {e}")))?;
    let branch = session.handle();

    let roles: Vec<MemberRole> = branch
        .query()
        .select(Query::<MemberRole> {
            this: Term::from(membership.this().clone()),
            role: Term::var("role"),
        })
        .perform(&joined.operator)
        .try_vec()
        .await
        .map_err(|e| InviteError::Io(format!("failed to read membership roles: {e:?}")))?;
    let stamps: Vec<InvitedVia> = branch
        .query()
        .select(Query::<InvitedVia> {
            this: Term::from(membership.this().clone()),
            invitation: Term::var("invitation"),
        })
        .perform(&joined.operator)
        .try_vec()
        .await
        .map_err(|e| InviteError::Io(format!("failed to read membership provenance: {e:?}")))?;

    // A member claiming their own invite is not provenance: it answers "how
    // did this member first get in", which is meaningless when the inviter is
    // the claimer.
    let self_invite = invitation.inviter.0 == member.this();
    let invitation_entity = invitation.this().clone();
    let mut transaction = branch
        .transaction()
        .assert(invitation)
        .assert(invitation_execution)
        .assert(membership.clone());
    if roles.is_empty() {
        transaction = transaction.assert(MemberRole::member(membership.this().clone()));
    }
    if stamps.is_empty() && !self_invite {
        transaction = transaction.assert(InvitedVia::new(
            membership.this().clone(),
            invitation_entity,
        ));
    }
    transaction
        .commit()
        .perform(&joined.operator)
        .await
        .map_err(|e| InviteError::Io(format!("failed to record membership: {e}")))?;
    Ok(())
}

/// Shorten a minted invite URL via the shortcut service on its own
/// origin: PUT the path + query, assemble `{origin}/@/{hash}` with the
/// seed fragment re-attached (the fragment never goes on the wire).
pub async fn shorten(url: &str) -> Result<String, InviteError> {
    let request = ShortcutRequest::new(url)
        .map_err(|e| InviteError::Io(format!("failed to derive shortcut: {e}")))?;
    let response = reqwest::Client::new()
        .put(request.endpoint.clone())
        .body(request.target.clone())
        .send()
        .await
        .map_err(|e| InviteError::Io(format!("shortcut PUT failed: {e}")))?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        return Err(InviteError::Io(format!(
            "shortcut PUT returned HTTP {status}: {detail}"
        )));
    }
    let hash = response
        .text()
        .await
        .map_err(|e| InviteError::Io(format!("shortcut response: {e}")))?;
    request
        .short_url(&hash)
        .map_err(|e| InviteError::Io(format!("failed to assemble short URL: {e}")))
}

/// Resolve a short link to the long invite URL it redirects to,
/// re-attaching the fragment the way a browser would.
async fn resolve_shortcut(short_url: &str) -> Result<String, InviteError> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| InviteError::Io(format!("failed to build HTTP client: {e}")))?;
    let response = client
        .get(short_url)
        .send()
        .await
        .map_err(|e| InviteError::Io(format!("failed to resolve invite link: {e}")))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(InviteError::InvalidInvite(
            "invite link not found; it may have been mistyped or removed".to_owned(),
        ));
    }
    if !response.status().is_redirection() {
        return Err(InviteError::InvalidInvite(format!(
            "invite link did not redirect (HTTP {})",
            response.status()
        )));
    }
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            InviteError::InvalidInvite("invite link redirect carries no Location".to_owned())
        })?;
    resolve_location(short_url, location).map_err(|e| InviteError::InvalidInvite(e.to_string()))
}

/// Generate an ephemeral Ed25519 signer with an extractable
/// seed. Mirrors [`tonk_worker`'s helper] — wasm's default
/// `Ed25519Signer::generate` produces a non-extractable
/// WebCrypto key whose seed can't be embedded in the invite
/// URL, so the wasm path opts in via [`ExtractableKey`]. Tonk
/// is native-only today, so the cfg gate is dormant; keeping it
/// in place lets a future `tonk-wasm` reuse this code path.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    mod when_deriving_a_base_url_from_a_remote {
        use super::*;

        #[dialog_common::test]
        fn it_replaces_the_access_service_path_with_join() {
            let base = base_url_for_remote("https://staging.tonk.xyz/ucan/").unwrap();
            assert_eq!(base, "https://staging.tonk.xyz/join");
        }

        #[dialog_common::test]
        fn it_handles_an_endpoint_with_no_path() {
            let base = base_url_for_remote("https://staging.tonk.xyz").unwrap();
            assert_eq!(base, "https://staging.tonk.xyz/join");
        }

        /// `/ucan` and `/ucan/` behave identically: joining an
        /// absolute path discards the base path entirely rather than
        /// resolving relative to its last segment (RFC 3986 §5.3).
        /// Pinned because it is the shape a reader is most likely to
        /// guess wrong about.
        #[dialog_common::test]
        fn it_treats_a_trailing_slash_as_irrelevant() {
            assert_eq!(
                base_url_for_remote("https://staging.tonk.xyz/ucan").unwrap(),
                base_url_for_remote("https://staging.tonk.xyz/ucan/").unwrap(),
            );
        }

        #[dialog_common::test]
        fn it_keeps_the_port_so_local_services_resolve() {
            let base = base_url_for_remote("http://127.0.0.1:8787/ucan/").unwrap();
            assert_eq!(base, "http://127.0.0.1:8787/join");
        }

        /// Credentials on a registered remote must not ride into a
        /// link that gets printed and pasted.
        #[dialog_common::test]
        fn it_strips_userinfo_from_the_endpoint() {
            let base = base_url_for_remote("https://user:secret@tonk.example/ucan/").unwrap();
            assert_eq!(base, "https://tonk.example/join");
        }

        #[dialog_common::test]
        fn it_rejects_an_endpoint_that_is_not_a_url() {
            assert!(base_url_for_remote("not a url").is_err());
        }
    }

    mod when_deciding_whether_to_shorten {
        use super::*;

        #[dialog_common::test]
        fn it_is_off_when_the_flag_is_passed() {
            assert!(!shorten_enabled(true));
        }
    }
}
