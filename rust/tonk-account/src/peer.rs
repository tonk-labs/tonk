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
    ACCESS_BRANCH, AddAddressError, Branch, ConnectError, ConnectedBranch, ConnectedReplica,
    PeersEnv, Repository, ResolveEnv, SiteAddress, Upstream, contact,
};
use dialog_storage::provider::storage::{CredentialStore, Storage};
use dialog_storage::resource::Resource;
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;
use dialog_varsig::Principal as _;
use std::fmt::Display;

/// The name the key of the system tonk runs as is kept under, in the
/// credential store of the profile directory. The system owns the storage
/// and the credential store, and grants its profiles the storage.
pub const SYSTEM_CREDENTIAL: &str = "tonk-system";

/// The name of the vault a peer's space records the account it acts for
/// in.
pub const ACCOUNT_VAULT: &str = "account";

/// The name the key guarding a profile's account before sign-in is kept
/// under, beside the profile: the account's custodian until the account
/// is handed over to the one the device signs in to.
pub const ACCOUNT_CUSTODIAN: &str = "tonk-account-custodian-v1";

/// The name the custodian of the account a peer acts for on `branch` is
/// kept under. Each account branch of a profile is its own account, so
/// each keeps its own custodian; the profile's [`ACCESS_BRANCH`] keeps the
/// name every profile had before accounts were branches.
pub fn custodian_name(branch: &str) -> String {
    if branch == ACCESS_BRANCH {
        ACCOUNT_CUSTODIAN.to_string()
    } else {
        format!("{ACCOUNT_CUSTODIAN}:{branch}")
    }
}

/// Where the credential store of the directory at `directory` keeps the
/// key named `name` on the filesystem, for the few places that must look
/// without opening it: whether a key was ever kept, and what its file's
/// permissions are.
#[cfg(not(target_arch = "wasm32"))]
pub fn kept_key_path(directory: &std::path::Path, name: &str) -> std::path::PathBuf {
    directory
        .join("dialog.credential")
        .join("credential/key")
        .join(name)
}

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
    S: PeerSpace + Resource<Location, Error: Display>,
    CredentialStore<S>: Provider<storage_fx::Load> + Provider<storage_fx::Create>,
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

    match credentials.adopt_from(storage, location).await {
        Ok(Credential::Signer(signer)) => return Ok(signer),
        Ok(Credential::Verifier(_)) | Err(storage_fx::StorageError::NotFound(_)) => {}
        Err(error) => {
            return Err(failed(format!("failed to keep the profile's key: {error}")));
        }
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

/// Open the peer a profile is: its key opened from `credentials` under the
/// name `location` gives, its [home](home) space in `storage`, its records
/// in the home's [`ACCESS_BRANCH`], granted the storage by `system`,
/// spaces it names resolving against `base`, and [onboarded](onboard).
/// With `create` false, a profile that holds no key is refused rather
/// than given one.
pub async fn open_peer<S>(
    location: Location,
    base: Directory,
    storage: Storage<S>,
    credentials: &CredentialStore<S>,
    system: &SignerCredential,
    create: bool,
) -> Result<Peer<S>, PeerError>
where
    S: PeerSpace + Resource<Location, Error: Display>,
    CredentialStore<S>: Provider<storage_fx::Load> + Provider<storage_fx::Create>,
{
    open_peer_on(
        location,
        base,
        storage,
        credentials,
        system,
        create,
        ACCESS_BRANCH,
    )
    .await
}

/// Open the peer a profile is as it acts on the account branch `branch`:
/// as [`open_peer`], its records kept in the home's `branch`. Each account
/// branch of a profile is its own account, with its own account vault,
/// custody and delegations, so a profile on another account's branch
/// acts for that account and nothing it holds for another.
pub async fn open_peer_on<S>(
    location: Location,
    base: Directory,
    storage: Storage<S>,
    credentials: &CredentialStore<S>,
    system: &SignerCredential,
    create: bool,
    branch: &str,
) -> Result<Peer<S>, PeerError>
where
    S: PeerSpace + Resource<Location, Error: Display>,
    CredentialStore<S>: Provider<storage_fx::Load> + Provider<storage_fx::Create>,
{
    let credential = open_credential(&location, credentials, &storage, create).await?;
    record_location(&credential.did(), &location);
    let home = home(&location, &base, &credential.did(), &storage).await?;
    let peer = Peer::new(credential.clone())
        .at(home)
        .base(base)
        .space(Repository::from(credential.did()).branch(branch))
        .with(storage)
        .grant(Allowance::storage(system))
        .build()
        .await?;
    onboard(&peer)
        .await
        .map_err(|error| PeerError::State(error.to_string()))?;
    Ok(peer)
}

/// Where the home space of the profile `profile` is: among the spaces it
/// names, under its own DID, like any other space. The profile's name
/// finds its key, and the key says where its space is.
///
/// A profile from before that keeps its home under its name, at
/// `location`, and goes on living there: a store cannot be renamed, and
/// what the profile holds is in it. A space of that name that is another
/// key's is not this profile's home, and is left alone.
async fn home<S>(
    location: &Location,
    base: &Directory,
    profile: &dialog_varsig::Did,
    storage: &Storage<S>,
) -> Result<Location, PeerError>
where
    S: PeerSpace + Resource<Location, Error: Display>,
{
    let named = Subject::from(did!("local:storage"))
        .attenuate(storage_fx::Storage)
        .attenuate(location.clone())
        .load()
        .perform(storage)
        .await;
    match named {
        Ok(held) if held.did() == *profile => Ok(location.clone()),
        Ok(_) | Err(storage_fx::StorageError::NotFound(_)) => {
            Ok(Location::new(base.clone(), profile.to_string()))
        }
        Err(error) => Err(PeerError::Open(format!(
            "failed to look for the profile's space: {error}"
        ))),
    }
}

/// Where each profile opened in this process was opened from, by its DID:
/// the name its key is kept under, and the directory the keys kept beside
/// it are in. A profile from before homes were named by DID has its space
/// there too.
fn locations() -> &'static std::sync::Mutex<std::collections::HashMap<String, Location>> {
    static LOCATIONS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, Location>>,
    > = std::sync::OnceLock::new();
    LOCATIONS.get_or_init(Default::default)
}

fn record_location(profile: &dialog_varsig::Did, location: &Location) {
    locations()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .insert(profile.to_string(), location.clone());
}

/// Where the profile `profile` opened in this process lives.
fn location_of(profile: &dialog_varsig::Did) -> Result<Location, CredentialError> {
    locations()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .get(&profile.to_string())
        .cloned()
        .ok_or_else(|| {
            CredentialError::NotFound(format!("the profile {profile} was not opened here"))
        })
}

/// Where the key `name` kept beside the profile `profile` lives: in the
/// credential store of the profile's directory, under a name of the
/// profile's own.
fn kept_key(profile: &dialog_varsig::Did, name: &str) -> Result<Location, CredentialError> {
    let directory = location_of(profile)?.directory;
    let owner = blake3::hash(profile.to_string().as_bytes()).to_hex();
    Ok(Location::new(directory, format!("{name}-{owner}")))
}

/// Move the key `name` a profile from before keys left its space kept
/// there into the credential store beside it, where [`open_kept_key`]
/// finds it, and remove it from the space. A record there holding only
/// the key's public half is a key retired before, and is removed without
/// being kept.
///
/// Safe to run on every open, and to stop anywhere: the key is kept
/// before it leaves the space, and one the store already keeps is not
/// replaced.
pub async fn adopt_kept_key<S>(
    profile: &dialog_varsig::Did,
    name: &str,
) -> Result<(), CredentialError>
where
    S: PeerSpace + Resource<Location, Error: Display>,
    CredentialStore<S>: Provider<storage_fx::Create>,
{
    let space = match S::load(&location_of(profile)?).await {
        Ok(space) => space,
        Err(error) if S::is_not_found(&error) => return Ok(()),
        Err(error) => return Err(CredentialError::Storage(error.to_string())),
    };
    let held = match profile
        .clone()
        .credential()
        .key(name)
        .load()
        .perform(&space)
        .await
    {
        Ok(held) => held,
        Err(error) if is_missing(&error) => return Ok(()),
        Err(error) => return Err(error),
    };
    if let Credential::Signer(_) = &held {
        let kept = Subject::from(did!("local:storage"))
            .attenuate(storage_fx::Storage)
            .attenuate(kept_key(profile, name)?)
            .create(held)
            .perform(&CredentialStore::<S>::new())
            .await;
        match kept {
            Ok(_) | Err(storage_fx::StorageError::AlreadyExists(_)) => {}
            Err(error) => return Err(CredentialError::Storage(error.to_string())),
        }
    }
    profile
        .clone()
        .credential()
        .key(name)
        .retract()
        .perform(&space)
        .await
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
/// creates the [`ACCOUNT_VAULT`], guarded by the [`ACCOUNT_CUSTODIAN`]
/// kept beside the profile, and the account delegates to the peer. The
/// peer holds no copy of the account's key.
pub async fn onboard<S>(peer: &Peer<S>) -> Result<(), CredentialError>
where
    S: PeerSpace,
    CredentialStore<S>: Provider<storage_fx::Load> + Provider<storage_fx::Create>,
{
    if peer.authority().await.is_ok() {
        return Ok(());
    }
    let custodian = open_kept_key::<S>(&peer.did(), &custodian_name(peer.state().name()), true)
        .await?
        .ok_or_else(|| CredentialError::NotFound("no account custodian".into()))?;
    let account = peer
        .state()
        .vault(ACCOUNT_VAULT)
        .create()
        .perform(peer)
        .await?;
    account.add(custodian.did()).perform(peer).await?;
    account.delegate(peer.did()).perform(peer).await
}

/// Copy the site secrets `from` keeps under a name `keep` accepts to `to`,
/// skipping one `to` keeps already, and answer how many were copied. A site
/// secret is sealed to the peer that keeps it, so each is opened by `from`
/// and sealed again by `to`; `from` keeps its own.
///
/// What a profile's account branch kept while every branch shared one peer
/// follows the branch to the peer that now acts on it.
pub async fn copy_site_secrets<S>(
    from: &Peer<S>,
    to: &Peer<S>,
    keep: impl Fn(&str) -> bool,
) -> Result<usize, CredentialError>
where
    S: PeerSpace,
{
    let failed = |error: &dyn Display| CredentialError::Storage(error.to_string());
    from.state().refresh(from).await.map_err(|e| failed(&e))?;
    let names: Vec<String> =
        dialog_repository::secrets::secrets_of(from.state(), &from.did(), from)
            .await
            .map_err(|e| failed(&e))?
            .into_iter()
            .map(|(name, _)| name)
            .filter(|name| keep(name))
            .collect();
    let mut copied = 0;
    for name in names {
        if to
            .secrets()
            .site(name.as_str())
            .load::<Vec<u8>>()
            .perform(to)
            .await
            .is_ok()
        {
            continue;
        }
        let secret = from
            .secrets()
            .site(name.as_str())
            .load::<Vec<u8>>()
            .perform(from)
            .await?;
        to.secrets()
            .site(name.as_str())
            .save(secret)
            .perform(to)
            .await?;
        copied += 1;
    }
    Ok(copied)
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

/// The DID of the service reached at `address`: the `did:web` its host
/// publishes a document for, whatever path the service is served under.
pub fn service_did(address: &SiteAddress) -> Result<dialog_varsig::Did, ConnectReplicaError> {
    let authority = service_name(address)?.replacen(':', "%3A", 1);
    format!("did:web:{authority}")
        .parse()
        .map_err(|_| ConnectReplicaError::NotAService(format!("{address:?}")))
}

/// The name the host knows the service reached at `address` by: the
/// host and port of its endpoint, the authority its `did:web` names.
///
/// Contact names are the host's, so one name is one peer: every
/// repository at a service, and every account signed in through it, is
/// reached through the same contact.
pub fn service_name(address: &SiteAddress) -> Result<String, ConnectReplicaError> {
    let SiteAddress::Ucan(ucan) = address else {
        return Err(ConnectReplicaError::NotAService(format!("{address:?}")));
    };
    let endpoint = url::Url::parse(ucan.endpoint())
        .map_err(|_| ConnectReplicaError::NotAService(ucan.endpoint().to_string()))?;
    let host = endpoint
        .host_str()
        .ok_or_else(|| ConnectReplicaError::NotAService(ucan.endpoint().to_string()))?;
    Ok(match endpoint.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    })
}

/// Connect to the replica of `subject` held by the service reached at
/// `address`, looking the service up among the host's contacts by its
/// [`service_name`].
///
/// A service the host does not know at `address` yet is recorded first,
/// under the DID it publishes and its name. A contact that already has
/// the address is left alone: recording commits to the host's state
/// even when nothing changes.
pub async fn connect<Env: PeersEnv>(
    address: SiteAddress,
    subject: dialog_varsig::Did,
    env: &Env,
) -> Result<ConnectedReplica, ConnectReplicaError> {
    let address = with_socket(address);
    let name = service_name(&address)?;
    if let Ok(replica) = contact(name.as_str())
        .connect()
        .repository(subject.clone())
        .open()
        .perform(env)
        .await
        && replica.addresses().contains(&address)
    {
        return Ok(replica);
    }
    let did = service_did(&address)?;
    contact(&did)
        .add_address(address)
        .name(name.as_str())
        .perform(env)
        .await?;
    Ok(contact(name)
        .connect()
        .repository(subject)
        .open()
        .perform(env)
        .await?)
}

/// `address` with its service's socket: the endpoint itself over `wss`,
/// or `ws` for a plain `http` one, where a tonk access service keeps each
/// space's socket. An address that names a socket keeps it, and one that
/// is not a service's is left as it is.
///
/// A service without a socket is still reached: what would have gone
/// over it goes as requests, and a watch there is refused, so the host
/// checks on its own schedule instead.
pub fn with_socket(address: SiteAddress) -> SiteAddress {
    let SiteAddress::Ucan(ucan) = &address else {
        return address;
    };
    if ucan.socket().is_some() {
        return address;
    }
    let endpoint = ucan.endpoint();
    let socket = if let Some(rest) = endpoint.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = endpoint.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        return address;
    };
    SiteAddress::Ucan(ucan.clone().with_socket(socket))
}

/// Why [`connect`] could not reach a replica.
#[derive(Debug, thiserror::Error)]
pub enum ConnectReplicaError {
    /// The address is not a tonk service's endpoint, so it names no
    /// service to look up.
    #[error("{0} is not the endpoint of a service")]
    NotAService(String),
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
/// [`ACCOUNT_VAULT`], opened through its [`ACCOUNT_CUSTODIAN`], is handed
/// over to the account's DID, so the account the peer acts for is the one
/// tonk signed in, not the one made up at onboarding. The custodian is
/// forgotten after: the account it guarded is gone. A peer already acting
/// for that account is left alone, and so is one whose custodian is gone,
/// which was handed over to another account before.
pub async fn hand_over<S>(peer: &Peer<S>, grant: &DelegationChain) -> Result<(), CredentialError>
where
    S: PeerSpace,
    CredentialStore<S>: Provider<storage_fx::Load>
        + Provider<storage_fx::Create>
        + Provider<credential_fx::Retract<Credential>>,
{
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
    let custodian_name = custodian_name(peer.state().name());
    let Some(custodian) = open_kept_key::<S>(&peer.did(), &custodian_name, false).await? else {
        return Ok(());
    };
    let current = match peer
        .state()
        .vault(ACCOUNT_VAULT)
        .load()
        .via(&custodian)
        .perform(peer)
        .await
    {
        Ok(current) => current,
        // The custodian guards an account the peer no longer acts for.
        Err(CredentialError::Withheld(_)) => return Ok(()),
        Err(error) => return Err(error),
    };
    current.hand_over(account).perform(peer).await?;
    forget_kept_key::<S>(&peer.did(), &custodian_name).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_credentials::{Ed25519Signer, Signer};

    /// A profile from before site secrets were sealed to vaults has them
    /// moved into its peer's site secrets, once: the old copies are gone,
    /// a secret the peer already keeps is not overwritten by a stale one,
    /// and running again moves nothing.
    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn it_moves_site_secrets_kept_in_the_profile_space() -> anyhow::Result<()> {
        use dialog_storage::provider::storage::NativeSpace;
        use dialog_storage::resource::Resource as _;

        let temp = tempfile::tempdir()?;
        let directory = Directory::At(temp.path().to_string_lossy().into_owned());
        let location = Location::new(directory.clone(), "legacy");
        let (credentials, system) = open_system::<NativeSpace>(directory.clone()).await?;
        let storage = Storage::<NativeSpace>::default().owned_by(system.did());
        let peer = open_peer(
            location.clone(),
            directory,
            storage,
            &credentials,
            &system,
            true,
        )
        .await?;

        // What a release before the move wrote: secrets in the profile's
        // own space, under the profile's DID.
        let space = NativeSpace::open(&location).await?;
        let old = || dialog_peer::CredentialHandle::new(peer.did());
        old()
            .site("tonk-local-root-v1")
            .save(b"root".to_vec())
            .perform(&space)
            .await?;
        old()
            .site("tonk-customer-v1")
            .save(b"stale".to_vec())
            .perform(&space)
            .await?;
        // A run stopped after copying this one, before retracting it, and
        // the peer's copy updated since.
        peer.secrets()
            .site("tonk-customer-v1")
            .save(b"current".to_vec())
            .perform(&peer)
            .await?;

        let sites: Vec<String> = ["tonk-local-root-v1", "tonk-customer-v1", "tonk-absent-v1"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(migrate_site_secrets(&peer, &location, &sites).await?, 1);
        assert_eq!(migrate_site_secrets(&peer, &location, &sites).await?, 0);

        let read = |site: &'static str| peer.secrets().site(site).load::<Vec<u8>>().perform(&peer);
        assert_eq!(read("tonk-local-root-v1").await?, b"root".to_vec());
        assert_eq!(read("tonk-customer-v1").await?, b"current".to_vec());
        for site in ["tonk-local-root-v1", "tonk-customer-v1"] {
            assert!(
                old()
                    .site(site)
                    .load::<Vec<u8>>()
                    .perform(&space)
                    .await
                    .is_err(),
                "the old copy of {site} is retracted"
            );
        }
        Ok(())
    }

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_dedicated_worker);

    /// The names of what `directory` holds.
    #[cfg(not(target_arch = "wasm32"))]
    fn entries(directory: &std::path::Path) -> Vec<String> {
        std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }

    /// A new profile's home is a space like any other: stored under the
    /// profile's DID among the spaces it names, not under the name its key
    /// is kept by. Opening it again by that name finds the same home.
    ///
    /// Native only, for the temporary directory it reads back: the browser
    /// keeps the same layout as databases, which the worker's
    /// `it_takes_a_new_space_into_custody_on_a_device_linked_without_a_key`
    /// opens a profile through.
    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn it_keeps_a_new_profile_home_under_its_did() -> anyhow::Result<()> {
        use dialog_storage::provider::storage::NativeSpace;

        let keys = tempfile::tempdir()?;
        let spaces = tempfile::tempdir()?;
        let directory = Directory::At(keys.path().to_string_lossy().into_owned());
        let base = Directory::At(spaces.path().to_string_lossy().into_owned());
        let open = || async {
            let (credentials, system) = open_system::<NativeSpace>(directory.clone()).await?;
            open_peer(
                Location::new(directory.clone(), "tonk"),
                base.clone(),
                Storage::<NativeSpace>::default().owned_by(system.did()),
                &credentials,
                &system,
                true,
            )
            .await
        };

        let peer = open().await?;
        assert_eq!(entries(spaces.path()), vec![peer.did().to_string()]);
        assert!(
            !entries(keys.path()).contains(&"tonk".to_string()),
            "the profile got a space under its name: {:?}",
            entries(keys.path())
        );

        let again = open().await?;
        assert_eq!(again.did(), peer.did());
        assert_eq!(again.authority().await?, peer.authority().await?);
        assert_eq!(entries(spaces.path()), vec![peer.did().to_string()]);
        Ok(())
    }

    /// A profile from before homes were named by DID keeps its home under
    /// its name: it opens as the identity it had, in the space that holds
    /// what it kept, and gets no second home under its DID.
    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn it_keeps_a_profile_home_that_was_named() -> anyhow::Result<()> {
        use dialog_storage::provider::storage::NativeSpace;
        use dialog_storage::resource::Resource as _;

        let keys = tempfile::tempdir()?;
        let spaces = tempfile::tempdir()?;
        let directory = Directory::At(keys.path().to_string_lossy().into_owned());
        let base = Directory::At(spaces.path().to_string_lossy().into_owned());
        let location = Location::new(directory.clone(), "tonk");

        // What a release before the credential store left: the profile's
        // key in the space under its name.
        let key = Ed25519Signer::generate().await?;
        let held = Credential::Signer(SignerCredential::from(key.clone()));
        Subject::from(key.did())
            .credential()
            .key(credential_fx::SELF)
            .save(held)
            .perform(&NativeSpace::open(&location).await?)
            .await?;

        let (credentials, system) = open_system::<NativeSpace>(directory.clone()).await?;
        let peer = open_peer(
            location,
            base,
            Storage::<NativeSpace>::default().owned_by(system.did()),
            &credentials,
            &system,
            false,
        )
        .await?;

        assert_eq!(peer.did(), key.did());
        assert!(entries(keys.path()).contains(&"tonk".to_string()));
        assert!(
            entries(spaces.path()).is_empty(),
            "the profile got a second home: {:?}",
            entries(spaces.path())
        );
        Ok(())
    }

    fn service(endpoint: &str) -> SiteAddress {
        SiteAddress::from(dialog_remote_ucan::UcanAddress::new(endpoint))
    }

    /// A service is named by the authority of its endpoint, whatever
    /// path it is served under, and its DID is the `did:web` of that
    /// authority, a port percent-encoded so it is not read as a path.
    #[dialog_common::test]
    fn it_names_a_service_by_its_host() -> anyhow::Result<()> {
        let hosted = service("https://tonk.network/ucan");
        assert_eq!(service_name(&hosted)?, "tonk.network");
        assert_eq!(service_did(&hosted)?.to_string(), "did:web:tonk.network");
        assert_eq!(
            service_name(&service("https://tonk.network/sync"))?,
            "tonk.network"
        );

        let local = service("http://localhost:8090/ucan");
        assert_eq!(service_name(&local)?, "localhost:8090");
        assert_eq!(service_did(&local)?.to_string(), "did:web:localhost%3A8090");

        assert!(matches!(
            service_name(&service("not a url")),
            Err(ConnectReplicaError::NotAService(_))
        ));
        Ok(())
    }

    /// A service is reached at its endpoint's socket too: over `wss` for
    /// an `https` endpoint and `ws` for a plain `http` one, on the same
    /// path. A socket an address names already is kept.
    #[dialog_common::test]
    fn it_reaches_a_service_at_its_socket_too() {
        let socket_of = |address: SiteAddress| match with_socket(address) {
            SiteAddress::Ucan(ucan) => ucan.socket().map(str::to_string),
            _ => None,
        };
        assert_eq!(
            socket_of(service("https://tonk.network/ucan/")),
            Some("wss://tonk.network/ucan/".into())
        );
        assert_eq!(
            socket_of(service("http://localhost:8090/ucan/")),
            Some("ws://localhost:8090/ucan/".into())
        );
        let named = SiteAddress::Ucan(
            dialog_remote_ucan::UcanAddress::new("https://tonk.network/ucan/")
                .with_socket("wss://elsewhere.example/"),
        );
        assert_eq!(socket_of(named), Some("wss://elsewhere.example/".into()));
    }

    /// Every repository at a service is reached through one contact,
    /// looked up by the service's name and recorded under its `did:web`;
    /// connecting again records nothing new.
    ///
    /// Native only, like the tests above, for the temporary profile
    /// directory: the browser opens the same peer over OPFS, and the
    /// worker's `it_takes_a_new_space_into_custody_on_a_device_linked_without_a_key`
    /// connects through this path there.
    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn it_reaches_every_repository_at_a_service_through_one_contact() -> anyhow::Result<()> {
        use dialog_storage::provider::storage::NativeSpace;

        let temp = tempfile::tempdir()?;
        let directory = Directory::At(temp.path().to_string_lossy().into_owned());
        let (credentials, system) = open_system::<NativeSpace>(directory.clone()).await?;
        let peer = open_peer(
            Location::new(directory.clone(), "contacts"),
            directory,
            Storage::<NativeSpace>::default().owned_by(system.did()),
            &credentials,
            &system,
            true,
        )
        .await?;
        let ucan = service("https://tonk.network/ucan");
        let sync = service("https://tonk.network/sync");
        let first = Ed25519Signer::generate().await?.did();
        let second = Ed25519Signer::generate().await?.did();

        connect(ucan.clone(), first.clone(), &peer).await?;
        let state = peer.state().revision();
        connect(ucan.clone(), first.clone(), &peer).await?;
        assert_eq!(
            peer.state().revision(),
            state,
            "a known service is not recorded again"
        );
        connect(sync.clone(), second.clone(), &peer).await?;

        let by_name = peer
            .contact("tonk.network")
            .connect()
            .repository(second)
            .open()
            .perform(&peer)
            .await?;
        assert!(by_name.addresses().contains(&with_socket(ucan.clone())));
        assert!(by_name.addresses().contains(&with_socket(sync.clone())));
        let by_did = peer
            .contact(service_did(&ucan)?)
            .connect()
            .repository(first)
            .open()
            .perform(&peer)
            .await?;
        assert_eq!(by_did.addresses(), by_name.addresses());
        Ok(())
    }

    /// Signing in hands the account made up at onboarding over to the
    /// account tonk signed in, forgets the custodian that guarded it, and
    /// doing it again changes nothing.
    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn it_hands_the_peer_account_over_at_sign_in() -> anyhow::Result<()> {
        use dialog_storage::provider::storage::NativeSpace;

        let temp = tempfile::tempdir()?;
        let directory = Directory::At(temp.path().to_string_lossy().into_owned());
        let (credentials, system) = open_system::<NativeSpace>(directory.clone()).await?;
        let peer = open_peer(
            Location::new(directory.clone(), "signing-in"),
            directory,
            Storage::<NativeSpace>::default().owned_by(system.did()),
            &credentials,
            &system,
            true,
        )
        .await?;
        let onboarded = peer.authority().await?;
        let account = Ed25519Signer::generate().await?;
        let grant =
            crate::delegations::mint_account_union(&Signer::from(account.clone()), &peer.did())
                .await?;
        assert_ne!(onboarded, account.did());

        hand_over(&peer, &grant).await?;
        assert_eq!(peer.authority().await?, account.did());
        assert!(
            open_kept_key::<NativeSpace>(&peer.did(), ACCOUNT_CUSTODIAN, false)
                .await?
                .is_none(),
            "the custodian of the onboarded account is forgotten"
        );

        hand_over(&peer, &grant).await?;
        assert_eq!(peer.authority().await?, account.did());
        Ok(())
    }
}

/// Make `target` what `branch` pulls from and pushes to, in place of any
/// branch at a peer it tracked before: an account whose link moved is
/// followed at its new place, and the old one is no longer synced with.
pub async fn repoint_upstream<Env: ResolveEnv>(
    branch: &Branch,
    target: &ConnectedBranch,
    env: &Env,
) -> Result<(), RepointError> {
    for upstream in branch.upstreams().iter() {
        let Upstream::Remote {
            remote,
            branch: name,
            ..
        } = upstream
        else {
            continue;
        };
        if remote.same(target.repository()) && name == target.name() {
            continue;
        }
        let previous = remote.branch(name.as_str()).open().perform(env).await?;
        branch.unset_upstream(&previous).perform(env).await?;
    }
    branch.set_upstream(target).perform(env).await?;
    Ok(())
}

/// Why [`repoint_upstream`] could not repoint a branch.
#[derive(Debug, thiserror::Error)]
pub enum RepointError {
    /// The upstream tracked before could not be opened.
    #[error(transparent)]
    Open(#[from] dialog_repository::OpenRemoteBranchError),
    /// The upstreams could not be recorded.
    #[error(transparent)]
    Record(#[from] dialog_repository::SetUpstreamError),
}

/// Move the site secrets a profile from before site secrets were sealed to
/// vaults kept in its space at `location` into `peer`'s site secrets, for
/// each of `sites`, and retract the old copies. Answers how many moved.
///
/// Safe to run on every open, and to stop anywhere: a secret the peer
/// already keeps is not overwritten, so a run stopped between copying and
/// retracting only retracts on the next, and one with nothing left to
/// move changes nothing.
pub async fn migrate_site_secrets<S>(
    peer: &Peer<S>,
    location: &Location,
    sites: &[String],
) -> Result<usize, CredentialError>
where
    S: PeerSpace,
{
    let space = match S::load(location).await {
        Ok(space) => space,
        Err(error) if S::is_not_found(&error) => return Ok(0),
        Err(error) => return Err(CredentialError::Storage(error.to_string())),
    };
    let old = || dialog_peer::CredentialHandle::new(peer.did());
    let mut moved = 0;
    for site in sites {
        let bytes = match old()
            .site(site.as_str())
            .load::<Vec<u8>>()
            .perform(&space)
            .await
        {
            Ok(bytes) => bytes,
            Err(error) if is_missing(&error) => continue,
            Err(error) => return Err(error),
        };
        let kept = match peer
            .secrets()
            .site(site.as_str())
            .load::<Vec<u8>>()
            .perform(peer)
            .await
        {
            Ok(_) => true,
            Err(error) if is_missing(&error) => false,
            Err(error) => return Err(error),
        };
        if !kept {
            peer.secrets()
                .site(site.as_str())
                .save(bytes)
                .perform(peer)
                .await?;
            moved += 1;
        }
        old().site(site.as_str()).retract().perform(&space).await?;
    }
    Ok(moved)
}

/// Whether `error` says a secret is absent: a store reports it as not
/// found, a filesystem as a missing file.
fn is_missing(error: &CredentialError) -> bool {
    match error {
        CredentialError::NotFound(_) => true,
        CredentialError::Storage(message) => {
            message.contains("No such file or directory") || message.contains("not found")
        }
        CredentialError::Corrupted(_) | CredentialError::Withheld(_) => false,
    }
}
