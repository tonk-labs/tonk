//! AWS Signature V4 pre-signed URL generation for R2.
//!
//! This module wraps the `dialog-s3-credentials` crate with R2-specific configuration.

use dialog_s3_credentials::{Address, AuthorizationError, Authorizer, Checksum, Credentials, RequestInfo};

/// Pre-signed URL result
#[derive(Debug, Clone)]
pub struct PresignedUrl {
    pub url: String,
    /// Headers the client must include when following the redirect
    pub headers: Vec<(String, String)>,
}

/// Errors during pre-signing
#[derive(Debug, thiserror::Error)]
pub enum PresignError {
    #[error("Authorization error: {0}")]
    Authorization(#[from] AuthorizationError),
    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),
}

/// Configuration for R2 pre-signing
pub struct R2Config {
    pub account_id: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub bucket: String,
}

/// HTTP method for the pre-signed URL
#[derive(Debug, Clone, Copy)]
pub enum Method {
    Get,
    Put,
}

impl Method {
    fn as_str(&self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Put => "PUT",
        }
    }
}

/// Generate a pre-signed URL for R2.
///
/// # Arguments
///
/// * `config` - R2 configuration (credentials, bucket, etc.)
/// * `method` - HTTP method (GET or PUT)
/// * `key` - Object key (path within bucket)
/// * `expires_in_secs` - URL validity duration in seconds
/// * `checksum` - Optional SHA256 checksum for upload integrity verification
///
/// # Returns
///
/// A `PresignedUrl` containing the URL and any required headers.
pub async fn presign_url(
    config: &R2Config,
    method: Method,
    key: &str,
    expires_in_secs: u64,
    checksum: Option<Checksum>,
) -> Result<PresignedUrl, PresignError> {
    // R2 endpoint format: https://{account_id}.r2.cloudflarestorage.com
    let endpoint = format!(
        "https://{}.r2.cloudflarestorage.com",
        config.account_id
    );

    // Create address with R2-specific configuration
    // R2 always uses "auto" region and path-style URLs
    let address = Address::new(&endpoint, "auto", &config.bucket);

    // Create credentials with the address
    let credentials = Credentials::new(
        address,
        &config.access_key_id,
        &config.secret_access_key,
    )?;

    // Build the URL for the key
    let url = credentials.build_url(key)?;

    // Create request info
    let request = RequestInfo {
        method: method.as_str(),
        url,
        region: "auto".to_string(),
        checksum,
        acl: None,
        expires: expires_in_secs,
        time: chrono::Utc::now(),
        service: "s3".to_string(),
    };

    let auth = credentials.authorize(&request).await?;

    Ok(PresignedUrl {
        url: auth.url.to_string(),
        headers: auth.headers,
    })
}

// Tests require tokio which is only available with test-client feature
#[cfg(all(test, feature = "test-client"))]
mod tests {
    use super::*;
    use dialog_s3_credentials::Hasher;

    #[tokio::test]
    async fn test_presign_url_structure() {
        let config = R2Config {
            account_id: "test123".into(),
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            bucket: "test-bucket".into(),
        };

        let result = presign_url(&config, Method::Get, "test.txt", 3600, None).await;
        assert!(result.is_ok());

        let url = result.unwrap();
        assert!(url.url.contains("X-Amz-Algorithm=AWS4-HMAC-SHA256"));
        assert!(url.url.contains("X-Amz-Signature="));
        assert!(url.url.contains("test123.r2.cloudflarestorage.com"));
        assert!(url.url.contains("test-bucket"));
        assert!(url.url.contains("test.txt"));
    }

    #[tokio::test]
    async fn test_presign_url_with_nested_key() {
        let config = R2Config {
            account_id: "test123".into(),
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            bucket: "tonk-spaces".into(),
        };

        let key = "did:key:z6MkTest/abc123def456";
        let result = presign_url(&config, Method::Put, key, 3600, None).await;
        assert!(result.is_ok());

        let url = result.unwrap();
        // Verify the key is in the URL
        assert!(url.url.contains("tonk-spaces"));
        assert!(url.url.contains("abc123def456"));
    }

    #[tokio::test]
    async fn test_presign_url_with_checksum() {
        let config = R2Config {
            account_id: "test123".into(),
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            bucket: "test-bucket".into(),
        };

        let checksum = Hasher::Sha256.checksum(b"test content");
        let result = presign_url(&config, Method::Put, "test.txt", 3600, Some(checksum)).await;
        assert!(result.is_ok());

        let presigned = result.unwrap();
        // URL should include checksum in signed headers
        assert!(presigned.url.contains("x-amz-checksum-sha256"));
        // Headers should include the checksum for client to send
        assert!(
            presigned
                .headers
                .iter()
                .any(|(k, _)| k == "x-amz-checksum-sha256")
        );
    }
}
