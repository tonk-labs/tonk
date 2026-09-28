//! Opening the peer a tonk profile is.
//!
//! A profile is a dialog [`Peer`]: its key is kept in a
//! [`CredentialStore`], its spaces live in a [`Storage`], and both belong
//! to the system tonk runs as on this device. The worker and the CLI open
//! their profiles the same way, so the steps live here once.

use dialog_capability::{Provider, Subject, did};
use dialog_common::ConditionalSync;
use dialog_credentials::{Credential, SignerCredential};
use dialog_effects::credential::{self as credential_fx, CredentialError, prelude::*};
use dialog_effects::storage::{self as storage_fx, Directory, Location, LocationExt as _};
use dialog_peer::{Allowance, OpenCredential, Peer, PeerError, PeerSpace, SpaceVaultExt as _};
use dialog_repository::{
    ACCESS_BRANCH, AddAddressError, Branch, ConnectError, ConnectedReplica, PeersEnv, Repository,
    SiteAddress, Upstream, contact, peer_did,
};
use dialog_storage::provider::storage::{CredentialStore, Storage};
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;
use dialog_varsig::Principal as _;

/// The name the key of the system tonk runs as is kept under, in the
/// credential store of the profile directory. The system owns the storage
/// and the credential store, and grants its profiles the storage.
pub const SYSTEM_CREDENTIAL: &str = "tonk-system";

/// The name of the vault a peer's space records the account it acts for
/// in.
pub const ACCOUNT_VAULT: &str = "account";

/// The name of the vault below [`ACCOUNT_VAULT`] whose members are the
/// account's peers, and where site credentials are kept.
pub const PEER_VAULT: &str = "peer";

/// Open the key of the system tonk runs as, from the credential store in
/// `directory`, and the store, owned by that system.
pub async fn open_system<S>(
    directory: Directory,
) -> Result<(CredentialStore<S>, SignerCredential), PeerError>
where
    S: PeerSpace,
    CredentialStore<S>: Provider<storage_fx::Load> + Provider<storage_fx::Create>,
{
    let credentials = CredentialStore::<S>::new();
    let opened = OpenCredential::open(SYSTEM_CREDENTIAL)
        .at(directory.clone())
        .perform(&credentials)
        .await;
    // Two processes opening the system for the first time race to create
    // its key; the one that loses loads the key the other created.
    let system = match opened {
        Ok(system) => system,
        Err(_) => OpenCredential::load(SYSTEM_CREDENTIAL)
            .at(directory)
            .perform(&credentials)
            .await
            .map_err(|error| PeerError::Open(format!("failed to open the system key: {error}")))?,
    };
    Ok((credentials.owned_by(system.did()), system))
}

/// Open the key of the profile at `location` from `credentials`: load it,
/// or, with `create`, generate and keep one when there is none.
///
/// A profile from before keys were kept apart has its key in the space at
/// `location`. That key is moved into `credentials`, and the space keeps
/// only its verifier, so the profile opens as the identity it always had.
pub async fn open_credential<S>(
    location: &Location,
    credentials: &CredentialStore<S>,
    storage: &Storage<S>,
    create: bool,
) -> Result<SignerCredential, PeerError>
where
    S: PeerSpace,
    CredentialStore<S>: Provider<storage_fx::Load> + Provider<storage_fx::Create>,
    Storage<S>: Provider<storage_fx::Load> + Provider<credential_fx::Save<Credential>>,
{
    let failed = |error: String| PeerError::Open(error);
    match OpenCredential::load(location.name.clone())
        .at(location.directory.clone())
        .perform(credentials)
        .await
    {
        Ok(credential) => return Ok(credential),
        Err(dialog_peer::IdentityError::NotFound) => {}
        Err(error) => return Err(failed(error.to_string())),
    }

    let at = Subject::from(did!("local:storage"))
        .attenuate(storage_fx::Storage)
        .attenuate(location.clone());
    if let Ok(Credential::Signer(signer)) = at.clone().load().perform(storage).await {
        at.create(Credential::Signer(signer.clone()))
            .perform(credentials)
            .await
            .map_err(|error| failed(format!("failed to keep the profile's key: {error}")))?;
        Subject::from(signer.did())
            .credential()
            .key(credential_fx::SELF)
            .save(Credential::Signer(signer.clone()))
            .perform(storage)
            .await
            .map_err(|error| failed(format!("failed to drop the profile's stored key: {error}")))?;
        return Ok(signer);
    }

    if !create {
        return Err(failed(format!("no profile key named {}", location.name)));
    }
    OpenCredential::open(location.name.clone())
        .at(location.directory.clone())
        .perform(credentials)
        .await
        .map_err(|error| failed(error.to_string()))
}

/// Open the peer a profile is: its key opened from `credentials`, its home
/// space at `location` in `storage`, its records in the home's
/// [`ACCESS_BRANCH`], granted the storage by `system`, spaces it names
/// resolving against `base`, and [onboarded](onboard). With `create`
/// false, a profile that holds no key is refused rather than given one.
pub async fn open_peer<S>(
    location: Location,
    base: Directory,
    storage: Storage<S>,
    credentials: &CredentialStore<S>,
    system: &SignerCredential,
    create: bool,
) -> Result<Peer<S>, PeerError>
where
    S: PeerSpace,
    CredentialStore<S>: Provider<storage_fx::Load> + Provider<storage_fx::Create>,
    Storage<S>: Provider<storage_fx::Load> + Provider<credential_fx::Save<Credential>>,
{
    let credential = open_credential(&location, credentials, &storage, create).await?;
    record_directory(&credential.did(), &location.directory);
    let peer = Peer::new(credential.clone())
        .at(location)
        .base(base)
        .space(Repository::from(credential.did()).branch(ACCESS_BRANCH))
        .with(storage)
        .grant(Allowance::storage(system))
        .build()
        .await?;
    onboard(&peer)
        .await
        .map_err(|error| PeerError::State(error.to_string()))?;
    Ok(peer)
}

/// The directory each profile opened in this process lives in, by its
/// DID: where the keys kept beside it are.
fn directories() -> &'static std::sync::Mutex<std::collections::HashMap<String, Directory>> {
    static DIRECTORIES: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, Directory>>,
    > = std::sync::OnceLock::new();
    DIRECTORIES.get_or_init(Default::default)
}

fn record_directory(profile: &dialog_varsig::Did, directory: &Directory) {
    directories()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .insert(profile.to_string(), directory.clone());
}

/// Where the key `name` kept beside the profile `profile` lives: in the
/// credential store of the profile's directory, under a name of the
/// profile's own.
fn kept_key(profile: &dialog_varsig::Did, name: &str) -> Result<Location, CredentialError> {
    let directory = directories()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .get(&profile.to_string())
        .cloned()
        .ok_or_else(|| {
            CredentialError::NotFound(format!("the profile {profile} was not opened here"))
        })?;
    let owner = blake3::hash(profile.to_string().as_bytes()).to_hex();
    Ok(Location::new(directory, format!("{name}-{owner}")))
}

/// Open the key `name` kept for the profile `profile` in the credential
/// store beside it: load it, or, with `create`, generate one when there is
/// none. The key is generated as the platform generates keys, so in the
/// browser it cannot be exported.
pub async fn open_kept_key<S>(
    profile: &dialog_varsig::Did,
    name: &str,
    create: bool,
) -> Result<Option<SignerCredential>, CredentialError>
where
    S: PeerSpace,
    CredentialStore<S>: Provider<storage_fx::Load> + Provider<storage_fx::Create>,
{
    let location = kept_key(profile, name)?;
    let credentials = CredentialStore::<S>::new();
    let loaded = OpenCredential::load(location.name.clone())
        .at(location.directory.clone())
        .perform(&credentials)
        .await;
    match loaded {
        Ok(key) => return Ok(Some(key)),
        Err(dialog_peer::IdentityError::NotFound) if !create => return Ok(None),
        Err(dialog_peer::IdentityError::NotFound) => {}
        Err(error) => return Err(CredentialError::Storage(error.to_string())),
    }
    OpenCredential::create(location.name)
        .at(location.directory)
        .perform(&credentials)
        .await
        .map(Some)
        .map_err(|error| CredentialError::Storage(error.to_string()))
}

/// Destroy the key `name` kept for the profile `profile`: a later
/// [`open_kept_key`] finds none.
pub async fn forget_kept_key<S>(
    profile: &dialog_varsig::Did,
    name: &str,
) -> Result<(), CredentialError>
where
    S: PeerSpace,
    CredentialStore<S>: Provider<storage_fx::Load> + Provider<credential_fx::Retract<Credential>>,
{
    let location = kept_key(profile, name)?;
    OpenCredential::forget(location.name)
        .at(location.directory)
        .perform(&CredentialStore::<S>::new())
        .await
        .map_err(|error| CredentialError::Storage(error.to_string()))
}

/// Onboard `peer`, unless its space already records an account: the space
/// creates the [`ACCOUNT_VAULT`] with the peer as its member, the account
/// delegates to the peer, and the peer becomes a member of
/// [`ACCOUNT_VAULT`] → [`PEER_VAULT`], where its site credentials are
/// kept.
pub async fn onboard<S: PeerSpace>(peer: &Peer<S>) -> Result<(), CredentialError> {
    if peer.authority().await.is_ok() {
        return Ok(());
    }
    let account = peer
        .state()
        .vault(ACCOUNT_VAULT)
        .create()
        .perform(peer)
        .await?;
    account.add(peer.did()).perform(peer).await?;
    account.delegate(peer.did()).perform(peer).await?;
    let peers = account.vault(PEER_VAULT).open().perform(peer).await?;
    peers.add(peer.did()).perform(peer).await
}

/// Mount the space of a repository whose key tonk does not hold, under
/// `location` in `storage`: the space keeps the repository's public
/// identity, and the peer acts on it only under delegations it holds.
///
/// A space a peer creates is created under a key it seals to its account,
/// so a repository known only by its DID (an account's, or one joined
/// through an invitation) is mounted in the storage directly.
pub async fn mount_verifier<S>(
    storage: &Storage<S>,
    location: Location,
    repository: &dialog_varsig::Did,
) -> Result<Credential, storage_fx::StorageError>
where
    S: Clone + ConditionalSync,
    Storage<S>: Provider<storage_fx::Create>,
{
    let verifier: dialog_credentials::Verifier = repository.to_string().parse().map_err(|_| {
        storage_fx::StorageError::Storage(format!("{repository} names no public key"))
    })?;
    Subject::from(did!("local:storage"))
        .attenuate(storage_fx::Storage)
        .attenuate(location)
        .create(Credential::from(verifier))
        .perform(storage)
        .await
}

/// Connect to the replica of `subject` held by the peer reached at
/// `address`, recording the address among the host's contacts first.
///
/// This is what a named remote of a repository was: an address and a
/// subject. The peer is picked out by the DID its address names, not by
/// a name: contact names are the host's, so a name like `origin` that
/// every repository used would name one peer for all of them.
pub async fn connect<Env: PeersEnv>(
    address: SiteAddress,
    subject: dialog_varsig::Did,
    env: &Env,
) -> Result<ConnectedReplica, ConnectReplicaError> {
    let peer = peer_did(&address)?;
    // Recording an address commits to the host's state even when it is
    // already there, so a contact that already has it is left alone.
    if let Ok(replica) = contact(&peer)
        .connect()
        .repository(subject.clone())
        .open()
        .perform(env)
        .await
        && replica.addresses().contains(&address)
    {
        return Ok(replica);
    }
    contact(&peer).add_address(address).perform(env).await?;
    Ok(contact(&peer)
        .connect()
        .repository(subject)
        .open()
        .perform(env)
        .await?)
}

/// Why [`connect`] could not reach a replica.
#[derive(Debug, thiserror::Error)]
pub enum ConnectReplicaError {
    /// The address names no peer.
    #[error(transparent)]
    Peer(#[from] dialog_repository::PeerError),
    /// The address could not be recorded among the host's contacts.
    #[error(transparent)]
    AddAddress(#[from] AddAddressError),
    /// The peer could not be connected to.
    #[error(transparent)]
    Connect(#[from] ConnectError),
}

/// The upstream `branch` pulls from, when it tracks one. A branch may pull
/// from several; tonk sets at most one, so the first is the one it set.
pub fn upstream(branch: &Branch) -> Option<Upstream> {
    branch.pulls().iter().next().cloned()
}

/// Hand `peer`'s account over to the account `grant` comes from: the
/// account → device powerline tonk holds once the device signs in.
///
/// The grant is retained where the peer proves from, and the peer's
/// [`ACCOUNT_VAULT`] is rotated to the account's DID, so the account the
/// peer acts for is the one tonk signed in, not the one dialog made up
/// at onboarding. A peer already acting for that account is left alone,
/// and so is one acting for another account whose key it does not hold.
pub async fn hand_over<S: PeerSpace>(
    peer: &Peer<S>,
    grant: &DelegationChain,
) -> Result<(), CredentialError> {
    let account = grant.issuer().clone();
    if peer.authority().await? == account {
        return Ok(());
    }
    peer.access()
        .save(UcanDelegation(grant.clone()))
        .perform(peer)
        .await
        .map_err(|error| CredentialError::Storage(error.to_string()))?;
    peer.state()
        .refresh(peer)
        .await
        .map_err(|error| CredentialError::Storage(error.to_string()))?;
    let current = match peer.state().vault(ACCOUNT_VAULT).load().perform(peer).await {
        Ok(current) => current,
        // The peer already acts for an account whose key it does not
        // hold: one it was handed over to before. Handing it over again
        // takes that account's key, so the peer keeps acting for it.
        Err(CredentialError::Withheld(_)) => return Ok(()),
        Err(error) => return Err(error),
    };
    current.rotate().to(account).perform(peer).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_credentials::{Ed25519Signer, Signer};
    use dialog_peer::helpers::test_peer;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_dedicated_worker);

    /// Signing in hands the account dialog made up at onboarding over to
    /// the account tonk signed in, and doing it again changes nothing.
    #[dialog_common::test]
    async fn it_hands_the_peer_account_over_at_sign_in() -> anyhow::Result<()> {
        let peer = test_peer().await;
        let onboarded = peer.authority().await?;
        let account = Ed25519Signer::generate().await?;
        let grant =
            crate::delegations::mint_account_union(&Signer::from(account.clone()), &peer.did())
                .await?;
        assert_ne!(onboarded, account.did());

        hand_over(&peer, &grant).await?;
        assert_eq!(peer.authority().await?, account.did());

        hand_over(&peer, &grant).await?;
        assert_eq!(peer.authority().await?, account.did());
        Ok(())
    }
}
