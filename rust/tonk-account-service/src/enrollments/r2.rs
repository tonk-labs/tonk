//! Cloudflare R2 enrollment store.
//!
//! Shares the chain bucket rather than taking a binding of its own: the
//! objects live under `enrollments/{credential_did}/{key}`, disjoint from
//! `chains/` and `spot-heads/`. One fewer binding is one fewer thing that
//! can be missing from a deploy.

use dialog_varsig::Did;
use worker::Bucket;

use super::{
    ENROLLMENT_PREFIX, EnrollmentStore, EnrollmentStoreError, MAX_CLAIMED, VerifiedEnrollment,
    object_key,
};
use crate::revocations::PutOutcome;

/// R2-backed enrollment store.
pub struct R2EnrollmentStore(Bucket);

impl R2EnrollmentStore {
    /// Wrap the chain bucket binding.
    pub fn new(bucket: Bucket) -> Self {
        Self(bucket)
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, EnrollmentStoreError> {
        let Some(object) = self
            .0
            .get(key)
            .execute()
            .await
            .map_err(|error| EnrollmentStoreError::Internal(error.to_string()))?
        else {
            return Ok(None);
        };
        let Some(body) = object.body() else {
            return Ok(None);
        };
        body.bytes()
            .await
            .map(Some)
            .map_err(|error| EnrollmentStoreError::Internal(error.to_string()))
    }
}

impl EnrollmentStore for R2EnrollmentStore {
    async fn put(
        &self,
        verified: &VerifiedEnrollment,
        bytes: &[u8],
    ) -> Result<PutOutcome, EnrollmentStoreError> {
        let key = object_key(verified);
        if self.get(&key).await?.is_some() {
            return Ok(PutOutcome::Existing);
        }
        self.0
            .put(key, bytes.to_vec())
            .execute()
            .await
            .map_err(|error| EnrollmentStoreError::Internal(error.to_string()))?;
        Ok(PutOutcome::Stored)
    }

    async fn claim(&self, credential: &Did) -> Result<Vec<Vec<u8>>, EnrollmentStoreError> {
        let prefix = format!("{ENROLLMENT_PREFIX}{credential}/");
        let listing = self
            .0
            .list()
            .prefix(prefix)
            .limit(MAX_CLAIMED as u32)
            .execute()
            .await
            .map_err(|error| EnrollmentStoreError::Internal(error.to_string()))?;

        let mut chains = Vec::new();
        for object in listing.objects() {
            if let Some(bytes) = self.get(&object.key()).await? {
                chains.push(bytes);
            }
        }
        Ok(chains)
    }
}
