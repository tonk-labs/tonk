use crate::error::{RelayError, Result};
use aws_sdk_s3::Client;
use aws_sdk_s3::primitives::ByteStream;
use std::time::SystemTime;

#[derive(Clone)]
pub struct S3Storage {
    client: Client,
    bucket: String,
    is_available: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl S3Storage {
    pub async fn new(bucket: String, region: String) -> Result<Self> {
        let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new(region))
            .load()
            .await;

        let client = Client::new(&config);

        let storage = Self {
            client,
            bucket,
            is_available: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };

        Ok(storage)
    }

    pub async fn health_check(&self) -> bool {
        if self.is_available.load(std::sync::atomic::Ordering::Relaxed) {
            return true;
        }

        match self
            .client
            .list_objects_v2()
            .bucket(&self.bucket)
            .max_keys(1)
            .send()
            .await
        {
            Ok(_) => {
                self.is_available
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                true
            }
            Err(e) => {
                tracing::error!("S3 health check failed: {}", e);
                false
            }
        }
    }

    pub async fn upload_bundle(&self, bundle_id: &str, data: Vec<u8>) -> Result<()> {
        if !self.health_check().await {
            return Err(RelayError::S3("S3 not available".to_string()));
        }

        let key = format!("bundles/{}.tonk", bundle_id);

        let byte_stream = ByteStream::from(data);

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(byte_stream)
            .content_type("application/octet-stream")
            .metadata(
                "uploadedAt",
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    .to_string(),
            )
            .send()
            .await
            .map_err(|e| RelayError::S3(format!("Failed to upload bundle: {}", e)))?;

        tracing::info!("Bundle uploaded successfully: {}", key);
        Ok(())
    }

    pub async fn download_bundle(&self, bundle_id: &str) -> Result<Vec<u8>> {
        if !self.health_check().await {
            return Err(RelayError::S3("S3 not available".to_string()));
        }

        let key = format!("bundles/{}.tonk", bundle_id);

        let response = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
            .map_err(|e| {
                if e.to_string().contains("NoSuchKey") {
                    RelayError::NotFound(format!("Bundle not found: {}", bundle_id))
                } else {
                    RelayError::S3(format!("Failed to download bundle: {}", e))
                }
            })?;

        let data = response
            .body
            .collect()
            .await
            .map_err(|e| RelayError::S3(format!("Failed to read bundle data: {}", e)))?;

        Ok(data.to_vec())
    }

    pub async fn bundle_exists(&self, bundle_id: &str) -> Result<bool> {
        if !self.health_check().await {
            return Err(RelayError::S3("S3 not available".to_string()));
        }

        let key = format!("bundles/{}.tonk", bundle_id);

        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(e) => {
                if e.to_string().contains("NotFound") || e.to_string().contains("NoSuchKey") {
                    Ok(false)
                } else {
                    Err(RelayError::S3(format!(
                        "Failed to check bundle existence: {}",
                        e
                    )))
                }
            }
        }
    }

    pub async fn get_bundle_metadata(&self, bundle_id: &str) -> Result<Option<BundleMetadata>> {
        if !self.health_check().await {
            return Err(RelayError::S3("S3 not available".to_string()));
        }

        let key = format!("bundles/{}.tonk", bundle_id);

        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
        {
            Ok(response) => Ok(Some(BundleMetadata {
                size: response.content_length().unwrap_or(0) as u64,
                last_modified: response.last_modified().map(|dt| {
                    SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(dt.secs() as u64)
                }),
            })),
            Err(e) => {
                if e.to_string().contains("NotFound") || e.to_string().contains("NoSuchKey") {
                    Ok(None)
                } else {
                    Err(RelayError::S3(format!(
                        "Failed to get bundle metadata: {}",
                        e
                    )))
                }
            }
        }
    }
}

pub struct BundleMetadata {
    pub size: u64,
    pub last_modified: Option<SystemTime>,
}
