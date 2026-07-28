//! Complete paginated R2 source for immutable revocation artifacts.

use std::collections::HashSet;
use worker::Bucket;

use super::{RevocationSource, SourceError, StoredArtifact};

const PREFIX: &str = "revocations/";

/// Read-only R2 source bound to the global revocation bucket.
pub struct R2RevocationSource(Bucket);

impl R2RevocationSource {
    /// Wrap the `REVOCATIONS` bucket binding.
    pub fn new(bucket: Bucket) -> Self {
        Self(bucket)
    }
}

impl RevocationSource for R2RevocationSource {
    async fn complete_listing(
        &self,
        seen: &HashSet<String>,
    ) -> Result<Vec<StoredArtifact>, SourceError> {
        let mut keys = Vec::new();
        let mut cursor = None;
        loop {
            let mut request = self.0.list().prefix(PREFIX);
            if let Some(cursor) = cursor {
                request = request.cursor(cursor);
            }
            let page = request
                .execute()
                .await
                .map_err(|error| SourceError(error.to_string()))?;
            keys.extend(page.objects().into_iter().map(|object| object.key()));
            if !page.truncated() {
                break;
            }
            cursor = page.cursor();
            if cursor.is_none() {
                return Err(SourceError(
                    "R2 returned a truncated revocation listing without a cursor".into(),
                ));
            }
        }

        let mut artifacts = Vec::new();
        for key in keys {
            let mut parts = key.split('/');
            let shape = (parts.next(), parts.next(), parts.next(), parts.next());
            let artifact_cid = match shape {
                (Some("revocations"), Some(target), Some(artifact), None)
                    if !target.is_empty() && !artifact.is_empty() =>
                {
                    artifact
                }
                _ => {
                    return Err(SourceError(format!(
                        "malformed revocation object key: {key}"
                    )));
                }
            };
            if seen.contains(artifact_cid) {
                continue;
            }
            let object = self
                .0
                .get(&key)
                .execute()
                .await
                .map_err(|error| SourceError(error.to_string()))?
                .ok_or_else(|| SourceError(format!("listed revocation disappeared: {key}")))?;
            let bytes = object
                .body()
                .ok_or_else(|| SourceError(format!("revocation object has no body: {key}")))?
                .bytes()
                .await
                .map_err(|error| SourceError(error.to_string()))?;
            artifacts.push(StoredArtifact { key, bytes });
        }
        Ok(artifacts)
    }
}
