//! Cloudflare R2-backed [`ChainStore`](crate::chains::ChainStore), for
//! production use. Objects are namespaced under `chains/{root_did}/{key}`.

use worker::Bucket;

use crate::chains::{ChainError, ChainStore};

/// Cloudflare R2-backed [`ChainStore`], for production use.
pub struct R2ChainStore(Bucket);

impl R2ChainStore {
    /// Wrap an R2 bucket binding.
    pub fn new(bucket: Bucket) -> Self {
        Self(bucket)
    }
}

/// The R2 object key namespacing `key` under `root_did`.
fn object_key(root_did: &str, key: &str) -> String {
    format!("chains/{root_did}/{key}")
}

impl ChainStore for R2ChainStore {
    async fn put(&self, root_did: &str, key: &str, bytes: &[u8]) -> Result<(), ChainError> {
        self.0
            .put(object_key(root_did, key), bytes.to_vec())
            .execute()
            .await
            .map_err(|err| ChainError::Internal(err.to_string()))?;
        Ok(())
    }

    async fn list(&self, root_did: &str) -> Result<Vec<String>, ChainError> {
        let prefix = format!("chains/{root_did}/");
        let objects = self
            .0
            .list()
            .prefix(prefix.clone())
            .execute()
            .await
            .map_err(|err| ChainError::Internal(err.to_string()))?;
        Ok(objects
            .objects()
            .into_iter()
            .filter_map(|object| {
                let key = object.key();
                key.strip_prefix(&prefix).map(str::to_string)
            })
            .collect())
    }

    async fn get(&self, root_did: &str, key: &str) -> Result<Option<Vec<u8>>, ChainError> {
        let Some(object) = self
            .0
            .get(object_key(root_did, key))
            .execute()
            .await
            .map_err(|err| ChainError::Internal(err.to_string()))?
        else {
            return Ok(None);
        };
        let Some(body) = object.body() else {
            return Ok(None);
        };
        let bytes = body
            .bytes()
            .await
            .map_err(|err| ChainError::Internal(err.to_string()))?;
        Ok(Some(bytes))
    }
}
