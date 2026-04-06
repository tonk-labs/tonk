//! `carry join <TOKEN>` — redeem an invite token to join a repository.
//!
//! Decodes the invite token, imports the membership credential, builds a
//! delegation chain from the membership to the local profile, saves it,
//! and — for v3 tokens — sets up the sync remote and pulls the latest data.
//!
//! ## Delegation chain
//!
//! The invite token contains a chain `repo → inviter_profile → membership`.
//! The membership credential is an ephemeral signer that acts as a bearer
//! capability. To complete the handoff we:
//!
//! 1. Save the token's chain under the membership DID so the proof store
//!    knows the path from the repo to the membership.
//! 2. Use the membership signer to re-delegate to our own profile DID.
//! 3. Save the extended chain under our profile.
//!
//! After this, the local operator (derived from the profile) can prove
//! authority over the repo subject for UCAN invocations.

use crate::invite_cmd;
use crate::remote_cmd::HIDDEN_BRANCH;
use crate::site::Site;
use anyhow::{Context, Result};
use dialog_credentials::credential::Credential;
use dialog_credentials::credential::export::CredentialExport;
use dialog_repository::profile::access::Access;
use dialog_varsig::Principal;
use std::path::Path;

/// Execute `carry join <token> [--repo <REPO>]`.
///
/// `profile_location`: `None` for production, `Some(loc)` for test isolation.
pub async fn execute(
    token: &str,
    site_flag: Option<&Path>,
    profile_location: Option<crate::identity_cmd::ProfileLocation>,
) -> Result<()> {
    // Decode the invite token (v2 or v3)
    let decoded = invite_cmd::decode_token(token)?;

    // Import the membership credential (must be a signer for re-delegation)
    let cred_export = CredentialExport::try_from(decoded.cred_bytes)
        .context("Invalid credential in invite token")?;
    let credential = Credential::import(cred_export)
        .await
        .context("Failed to import membership credential")?;

    let membership_signer = match credential {
        Credential::Signer(s) => s,
        Credential::Verifier(_) => {
            anyhow::bail!("Invite token contains a verifier credential; expected a signer");
        }
    };

    eprintln!("Membership credential: {}", membership_signer.did());

    // Resolve or create the .carry/ site
    let site = match Site::resolve(site_flag, profile_location.clone()).await {
        Ok(site) => site,
        Err(_) => {
            let parent = if let Some(p) = site_flag {
                if p.ends_with(".carry") {
                    p.parent()
                        .context("--repo .carry path has no parent")?
                        .to_path_buf()
                } else {
                    p.to_path_buf()
                }
            } else {
                std::env::current_dir().context("Failed to determine current directory")?
            };
            Site::init(&parent, profile_location, None).await?
        }
    };

    // 1. Mount the membership DID at a volatile location so the proof
    //    store can save and look up chains under it. Then save the
    //    token's chain (audience = membership DID).
    let membership_did = membership_signer.did();
    use dialog_capability::Policy;
    use dialog_capability::Subject;
    use dialog_capability::access::{Permit, Save};
    use dialog_capability::storage::Storage as StorageCap;
    use dialog_capability_ucan::Ucan;
    use dialog_repository::helpers::unique_location;
    use dialog_storage::provider::Address;

    let membership_loc = unique_location("membership");
    let mount_addr = {
        use dialog_capability::storage::Location;
        Location::of(&membership_loc).address().clone()
    };
    StorageCap::mount::<Address>(membership_did.clone(), mount_addr)
        .perform(&site.operator)
        .await
        .map_err(|e| anyhow::anyhow!("mount failed: {:?}", e))?;

    Subject::from(membership_did.clone())
        .attenuate(Permit)
        .invoke(Save::<Ucan>::new(decoded.chain))
        .perform(&site.operator)
        .await
        .context("Failed to save token delegation chain under membership DID")?;

    // 2. The membership credential re-delegates to our profile.
    //    This extends the chain: repo → inviter → membership → our_profile.
    //    We must claim against the REMOTE repo subject (from the token),
    //    not our local repo DID which is different.
    let membership_access = Access::new(&membership_signer);
    let remote_subject: dialog_capability::Capability<dialog_capability::Subject> = Subject::from(
        decoded
            .remote
            .as_ref()
            .map(|r| r.subject().clone())
            .unwrap_or_else(|| site.repo.did()),
    )
    .into();
    let extended_chain = membership_access
        .claim(remote_subject)
        .delegate(site.profile.did())
        .perform(&site.operator)
        .await
        .context("Failed to re-delegate from membership to local profile")?;

    // 3. Save the extended chain under our profile.
    site.profile
        .access()
        .save(extended_chain)
        .perform(&site.operator)
        .await
        .context("Failed to save extended delegation chain")?;

    eprintln!("Joined repository as {}", site.profile.did());

    // If the token includes a remote address (v3), wire up sync and pull.
    if let Some(remote_address) = decoded.remote {
        eprintln!(
            "Configuring sync remote (subject: {})...",
            remote_address.subject()
        );

        let remote = site
            .repo
            .remote("origin")
            .create(remote_address.site().clone())
            .subject(remote_address.subject().clone())
            .perform(&site.operator)
            .await
            .context("Failed to register remote from invite token")?;

        let remote_branch = remote
            .branch(HIDDEN_BRANCH)
            .open()
            .perform(&site.operator)
            .await
            .context("Failed to open remote branch")?;

        site.branch
            .set_upstream(remote_branch)
            .perform(&site.operator)
            .await
            .context("Failed to set upstream")?;

        match site.branch.pull().perform(&site.operator).await {
            Ok(Some(rev)) => {
                eprintln!("Pulled. Local is now at {}.", rev.tree());
            }
            Ok(None) => {
                eprintln!("Remote is empty; nothing to pull yet.");
            }
            Err(e) => {
                // Don't fail the join if the pull fails — the delegation
                // and remote are already saved. The user can retry.
                eprintln!("warning: pull failed ({}); run `carry pull` to retry.", e);
            }
        }
    }

    Ok(())
}
