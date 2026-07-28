//! Cloudflare R2 immutable revocation writer.

use worker::Bucket;

use super::{PutOutcome, RevocationStore, RevocationStoreError, object_key};
use tonk_identity::revocation::VerifiedRevocation;

/// R2-backed immutable revocation writer.
pub struct R2RevocationStore(Bucket);

impl R2RevocationStore {
    /// Wrap the dedicated global revocation bucket binding.
    pub fn new(bucket: Bucket) -> Self {
        Self(bucket)
    }
}

impl RevocationStore for R2RevocationStore {
    async fn put(
        &self,
        verified: &VerifiedRevocation,
        bytes: &[u8],
    ) -> Result<PutOutcome, RevocationStoreError> {
        let key = object_key(verified);
        if self
            .0
            .get(&key)
            .execute()
            .await
            .map_err(|error| RevocationStoreError::Internal(error.to_string()))?
            .is_some()
        {
            return Ok(PutOutcome::Existing);
        }
        self.0
            .put(key, bytes.to_vec())
            .execute()
            .await
            .map_err(|error| RevocationStoreError::Internal(error.to_string()))?;
        Ok(PutOutcome::Stored)
    }
}
