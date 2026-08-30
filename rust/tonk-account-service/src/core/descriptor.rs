//! Shared validation and set-if-absent policy for repository descriptors.

use crate::core::CeremonyError;

/// Validate signed descriptor hex and require its subject to equal `root_did`.
pub async fn validate_descriptor(
    descriptor_hex: &str,
    root_did: &str,
) -> Result<Vec<u8>, CeremonyError> {
    let bytes = hex::decode(descriptor_hex)
        .map_err(|_| CeremonyError::Invalid("repositoryDescriptor must be hex".to_string()))?;
    let descriptor = tonk_account::AccountRepositoryDescriptorV1::validate(&bytes)
        .await
        .map_err(|error| {
            CeremonyError::Invalid(format!("invalid repositoryDescriptor: {error}"))
        })?;
    if descriptor.account_subject().as_ref() != root_did {
        return Err(CeremonyError::Invalid(
            "repositoryDescriptor subject does not match the account root".to_string(),
        ));
    }
    Ok(descriptor.bytes().to_vec())
}
