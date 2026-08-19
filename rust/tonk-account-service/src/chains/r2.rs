//! Cloudflare R2-backed [`ChainStore`](crate::chains::ChainStore), for
//! production use. Immutable objects are namespaced under
//! `chains/{root_did}/{key}` and independent named/unnamed heads under
//! `spot-heads/{slot}/{root_did}/{subject_key}`.

use worker::Bucket;

use crate::chains::{ChainError, ChainStore, SpotHeadSlot};

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

fn head_key(root_did: &str, subject_key: &str, slot: SpotHeadSlot) -> String {
    format!("spot-heads/{}/{root_did}/{subject_key}", slot.as_str())
}

async fn list_prefix(bucket: &Bucket, prefix: &str) -> Result<Vec<String>, ChainError> {
    let mut keys = Vec::new();
    let mut cursor = None;
    loop {
        let mut request = bucket.list().prefix(prefix.to_string());
        if let Some(cursor) = cursor {
            request = request.cursor(cursor);
        }
        let objects = request
            .execute()
            .await
            .map_err(|error| ChainError::Internal(error.to_string()))?;
        keys.extend(
            objects
                .objects()
                .into_iter()
                .filter_map(|object| object.key().strip_prefix(prefix).map(str::to_string)),
        );
        if !objects.truncated() {
            break;
        }
        cursor = Some(objects.cursor().ok_or_else(|| {
            ChainError::Internal("truncated R2 listing omitted its cursor".to_string())
        })?);
    }
    keys.sort();
    Ok(keys)
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
        list_prefix(&self.0, &format!("chains/{root_did}/")).await
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

    async fn put_spot_head(
        &self,
        root_did: &str,
        subject_key: &str,
        slot: SpotHeadSlot,
        blob_key: &str,
    ) -> Result<(), ChainError> {
        self.0
            .put(
                head_key(root_did, subject_key, slot),
                blob_key.as_bytes().to_vec(),
            )
            .execute()
            .await
            .map_err(|error| ChainError::Internal(error.to_string()))?;
        Ok(())
    }

    async fn spot_head(
        &self,
        root_did: &str,
        subject_key: &str,
        slot: SpotHeadSlot,
    ) -> Result<Option<String>, ChainError> {
        let Some(object) = self
            .0
            .get(head_key(root_did, subject_key, slot))
            .execute()
            .await
            .map_err(|error| ChainError::Internal(error.to_string()))?
        else {
            return Ok(None);
        };
        let Some(body) = object.body() else {
            return Ok(None);
        };
        let bytes = body
            .bytes()
            .await
            .map_err(|error| ChainError::Internal(error.to_string()))?;
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|error| ChainError::Internal(format!("spot head is not UTF-8: {error}")))
    }

    async fn list_spot_heads(
        &self,
        root_did: &str,
        slot: SpotHeadSlot,
    ) -> Result<Vec<(String, String)>, ChainError> {
        let subjects = list_prefix(
            &self.0,
            &format!("spot-heads/{}/{root_did}/", slot.as_str()),
        )
        .await?;
        let mut heads = Vec::with_capacity(subjects.len());
        for subject in subjects {
            let blob = self
                .spot_head(root_did, &subject, slot)
                .await?
                .ok_or_else(|| {
                    ChainError::Internal(format!("listed spot head '{subject}' disappeared"))
                })?;
            heads.push((subject, blob));
        }
        Ok(heads)
    }

    async fn delete_namespace(&self, root_did: &str) -> Result<(), ChainError> {
        for prefix in [
            format!("chains/{root_did}/"),
            format!("spot-heads/named/{root_did}/"),
            format!("spot-heads/unnamed/{root_did}/"),
        ] {
            loop {
                let listed = self
                    .0
                    .list()
                    .prefix(prefix.clone())
                    .limit(1000)
                    .execute()
                    .await
                    .map_err(|error| ChainError::Internal(error.to_string()))?;
                let keys: Vec<_> = listed
                    .objects()
                    .into_iter()
                    .map(|object| object.key())
                    .collect();
                if keys.is_empty() {
                    break;
                }
                self.0
                    .delete_multiple(keys)
                    .await
                    .map_err(|error| ChainError::Internal(error.to_string()))?;
            }
        }
        Ok(())
    }
}
