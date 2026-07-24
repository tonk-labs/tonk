//! Checking a signed revocation before it is recorded.
//!
//! A revocation names the `root → device` delegation it withdraws and
//! is signed by an authority entitled to withdraw it. Authority scales
//! with blast radius: a device may revoke its own grant, and only the
//! account root may revoke another device's. The check classifies which
//! authority signed, and that classification is stored with the
//! artifact so consumers can filter on it.
//!
//! Verification happens here so garbage cannot be parked in a user's
//! namespace, and again at every consumer — a consumer that trusts this
//! service has gained nothing over trusting the registry.

use dialog_credentials::Ed25519KeyResolver;
use dialog_ucan_core::InvocationChain;
use dialog_ucan_core::promise::Promised;
use dialog_varsig::algorithm::eddsa::Ed25519Signature;

use crate::chains::ChainStore;
use crate::core::CeremonyError;
use crate::core::backup::chain_key;
use crate::store::{Account, Device};

/// The key prefix revocations live under, keeping them enumerable
/// separately from delegation chain backups in the same namespace.
pub const REVOCATION_PREFIX: &str = "revocations/";

/// The command a revocation invokes.
const REVOKE_COMMAND: [&str; 2] = ["ucan", "revoke"];

/// The argument naming the withdrawn delegation.
const REVOKE_ARGUMENT: &str = "revoke";

/// Which authority signed a revocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attestation {
    /// Signed by the account root: authoritative for any device of the
    /// account, and unforgeable by a device that holds only a grant.
    Root,
    /// Signed by the device revoking itself, under the grant it holds.
    Device,
}

impl Attestation {
    /// The stored form, used in the artifact key.
    pub fn as_str(&self) -> &'static str {
        match self {
            Attestation::Root => "root",
            Attestation::Device => "device",
        }
    }
}

/// Parse, verify and classify a revocation of `target`'s grant.
///
/// Returns the attestation level on success. A device-signed
/// revocation naming a device other than its issuer is rejected here —
/// that is the authority rule, and it is the reason this function
/// takes the target device rather than just its CID.
pub async fn check_revocation(
    bytes: &[u8],
    root_did: &str,
    target: &Device,
) -> Result<Attestation, CeremonyError> {
    let chain = InvocationChain::<Ed25519Signature>::try_from(bytes)
        .map_err(|err| CeremonyError::Invalid(format!("bad revocation container: {err}")))?;

    chain.verify(&Ed25519KeyResolver).await.map_err(|err| {
        CeremonyError::Unauthorized(format!("revocation failed to verify: {err}"))
    })?;

    let command: Vec<&str> = chain.command().0.iter().map(String::as_str).collect();
    if command.as_slice() != REVOKE_COMMAND {
        return Err(CeremonyError::Invalid(format!(
            "expected a {REVOKE_COMMAND:?} invocation, got {command:?}"
        )));
    }

    if chain.subject().to_string() != root_did {
        return Err(CeremonyError::Forbidden(
            "revocation subject is not this account's root".to_string(),
        ));
    }

    let Some(Promised::String(named)) = chain.arguments().get(REVOKE_ARGUMENT) else {
        return Err(CeremonyError::Invalid(
            "revocation must name the delegation it withdraws".to_string(),
        ));
    };
    if named != &target.delegation_cid {
        return Err(CeremonyError::Invalid(
            "revocation names a delegation other than the target device's".to_string(),
        ));
    }

    let issuer = chain.issuer().to_string();
    if issuer == root_did {
        return Ok(Attestation::Root);
    }
    if issuer == target.device_did {
        return Ok(Attestation::Device);
    }
    Err(CeremonyError::Forbidden(
        "only the account root may revoke another device".to_string(),
    ))
}

/// Store a verified revocation in the account's namespace, keyed by its
/// attestation level and content hash.
///
/// The set is append-only: revocation is monotone, so nothing here ever
/// deletes, and re-storing identical bytes is idempotent.
pub async fn put_revocation<C: ChainStore>(
    chains: &C,
    account: &Account,
    attestation: Attestation,
    bytes: &[u8],
) -> Result<String, CeremonyError> {
    let key = format!(
        "{REVOCATION_PREFIX}{}/{}",
        attestation.as_str(),
        chain_key(bytes)
    );
    chains.put(&account.root_did, &key, bytes).await?;
    Ok(key)
}

/// A stored revocation, as served to consumers.
pub struct StoredRevocation {
    /// The storage key, carrying the attestation level.
    pub key: String,
    /// Which authority signed it.
    pub attestation: String,
    /// The container bytes, hex-encoded.
    pub revocation_hex: String,
}

/// List every revocation stored under this account, newest-agnostic:
/// the set is unordered, and consumers verify each artifact themselves.
///
/// One `get` per key over an unscoped `list` — fine while revocation
/// sets are tiny. A prefix-scoped `ChainStore::list` is the fix if
/// they stop being tiny.
pub async fn list_revocations<C: ChainStore>(
    chains: &C,
    account: &Account,
) -> Result<Vec<StoredRevocation>, CeremonyError> {
    let keys = chains.list(&account.root_did).await?;
    let mut stored = Vec::new();
    for key in keys {
        let Some(rest) = key.strip_prefix(REVOCATION_PREFIX) else {
            continue;
        };
        let attestation = rest.split('/').next().unwrap_or_default().to_string();
        let Some(bytes) = chains.get(&account.root_did, &key).await? else {
            continue;
        };
        stored.push(StoredRevocation {
            key: key.clone(),
            attestation,
            revocation_hex: hex::encode(&bytes),
        });
    }
    Ok(stored)
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::store::DeviceStatus;
    use dialog_credentials::Ed25519Signer;
    use dialog_varsig::Principal;
    use tonk_identity::revocation::{mint_root_revocation, mint_self_revocation};

    /// The account root DID as the checker takes it.
    fn root_did(root: &Ed25519Signer) -> String {
        root.did().to_string()
    }

    const ROOT_PRF: [u8; 32] = [7u8; 32];
    const FOREIGN_PRF: [u8; 32] = [9u8; 32];
    const DEVICE_SEED: [u8; 32] = [11u8; 32];
    const OTHER_SEED: [u8; 32] = [12u8; 32];

    /// A root, one device under it, and that device's registry row.
    async fn fixture(
        root_prf: [u8; 32],
        device_seed: [u8; 32],
    ) -> (
        Ed25519Signer,
        Ed25519Signer,
        dialog_ucan_core::DelegationChain,
        Device,
    ) {
        let root = tonk_identity::derive::derive_root_signer(&root_prf)
            .await
            .unwrap();
        let device = Ed25519Signer::import(&device_seed).await.unwrap();
        let grant = tonk_identity::delegation::mint_device_delegation(root.clone(), &device.did())
            .await
            .unwrap();
        let row = Device {
            account_id: 1,
            device_did: device.did().to_string(),
            delegation_cid: grant.proof_cids()[0].to_string(),
            name: "test device".to_string(),
            status: DeviceStatus::Active,
            created_at: 0,
        };
        (root, device, grant, row)
    }

    #[dialog_common::test]
    async fn it_accepts_a_root_signed_revocation() {
        let (root, _, _, row) = fixture(ROOT_PRF, DEVICE_SEED).await;
        let bytes = mint_root_revocation(root.clone(), &row.delegation_cid)
            .await
            .unwrap();

        let attestation = check_revocation(&bytes, root_did(&root).as_str(), &row)
            .await
            .unwrap();

        assert_eq!(attestation, Attestation::Root);
    }

    #[dialog_common::test]
    async fn it_accepts_a_device_revoking_itself() {
        let (root, device, grant, row) = fixture(ROOT_PRF, DEVICE_SEED).await;
        let bytes = mint_self_revocation(device, &grant, &root.did())
            .await
            .unwrap();

        let attestation = check_revocation(&bytes, root_did(&root).as_str(), &row)
            .await
            .unwrap();

        assert_eq!(attestation, Attestation::Device);
    }

    #[dialog_common::test]
    async fn it_rejects_a_device_revoking_another_device() {
        let (root, device, grant, _) = fixture(ROOT_PRF, DEVICE_SEED).await;
        let (_, _, _, other_row) = fixture(ROOT_PRF, OTHER_SEED).await;
        let bytes = mint_self_revocation(device, &grant, &root.did())
            .await
            .unwrap();

        let result = check_revocation(&bytes, root_did(&root).as_str(), &other_row).await;

        assert!(
            matches!(result, Err(CeremonyError::Invalid(_))),
            "a self-revocation names its own grant, not another device's"
        );
    }

    #[dialog_common::test]
    async fn it_rejects_a_revocation_from_a_foreign_root() {
        let (_, _, _, row) = fixture(ROOT_PRF, DEVICE_SEED).await;
        let (foreign, _, _, _) = fixture(FOREIGN_PRF, OTHER_SEED).await;
        let bytes = mint_root_revocation(foreign, &row.delegation_cid)
            .await
            .unwrap();

        let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
            .await
            .unwrap();
        let result = check_revocation(&bytes, root_did(&root).as_str(), &row).await;

        assert!(matches!(result, Err(CeremonyError::Forbidden(_))));
    }

    #[dialog_common::test]
    async fn it_rejects_a_revocation_naming_the_wrong_delegation() {
        let (root, _, _, row) = fixture(ROOT_PRF, DEVICE_SEED).await;
        let (_, _, _, other_row) = fixture(ROOT_PRF, OTHER_SEED).await;
        let bytes = mint_root_revocation(root.clone(), &other_row.delegation_cid)
            .await
            .unwrap();

        let result = check_revocation(&bytes, root_did(&root).as_str(), &row).await;

        assert!(matches!(result, Err(CeremonyError::Invalid(_))));
    }
}
