//! AWS Signature V4 pre-signed URL generation for R2.
//!
//! This module wraps the `dialog-s3-credentials` crate with R2-specific configuration.

use dialog_s3_credentials::{AuthorizationError, Checksum, Credentials, Invocation};
use url::Url;

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

/// Internal request type implementing the Invocation trait.
struct R2Request {
    url: Url,
    method: Method,
    region: &'static str,
    expires: u64,
    checksum: Option<Checksum>,
}

impl Invocation for R2Request {
    fn method(&self) -> &'static str {
        self.method.as_str()
    }

    fn url(&self) -> &Url {
        &self.url
    }

    fn region(&self) -> &str {
        self.region
    }

    fn expires(&self) -> u64 {
        self.expires
    }

    fn checksum(&self) -> Option<&Checksum> {
        self.checksum.as_ref()
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
pub fn presign_url(
    config: &R2Config,
    method: Method,
    key: &str,
    expires_in_secs: u64,
    checksum: Option<Checksum>,
) -> Result<PresignedUrl, PresignError> {
    // Build R2 URL: https://{account_id}.r2.cloudflarestorage.com/{bucket}/{key}
    let url = Url::parse(&format!(
        "https://{}.r2.cloudflarestorage.com/{}/{}",
        config.account_id, config.bucket, key
    ))?;

    let request = R2Request {
        url,
        method,
        region: "auto", // R2 always uses "auto"
        expires: expires_in_secs,
        checksum,
    };

    let credentials = Credentials {
        access_key_id: config.access_key_id.clone(),
        secret_access_key: config.secret_access_key.clone(),
    };

    let auth = credentials.authorize(&request)?;

    Ok(PresignedUrl {
        url: auth.url.to_string(),
        headers: auth.headers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_s3_credentials::Hasher;

    #[test]
    fn test_presign_url_structure() {
        let config = R2Config {
            account_id: "test123".into(),
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            bucket: "test-bucket".into(),
        };

        let result = presign_url(&config, Method::Get, "test.txt", 3600, None);
        assert!(result.is_ok());

        let url = result.unwrap();
        assert!(url.url.contains("X-Amz-Algorithm=AWS4-HMAC-SHA256"));
        assert!(url.url.contains("X-Amz-Signature="));
        assert!(url.url.contains("test123.r2.cloudflarestorage.com"));
        assert!(url.url.contains("/test-bucket/test.txt"));
    }

    #[test]
    fn test_presign_url_with_nested_key() {
        let config = R2Config {
            account_id: "test123".into(),
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            bucket: "tonk-spaces".into(),
        };

        let key = "did:key:z6MkTest/abc123def456";
        let result = presign_url(&config, Method::Put, key, 3600, None);
        assert!(result.is_ok());

        let url = result.unwrap();
        // Verify the key is in the URL (colons get percent-encoded)
        assert!(
            url.url
                .contains("/tonk-spaces/did:key:z6MkTest/abc123def456")
        );
    }

    #[test]
    fn test_presign_url_with_checksum() {
        let config = R2Config {
            account_id: "test123".into(),
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            bucket: "test-bucket".into(),
        };

        let checksum = Hasher::Sha256.checksum(b"test content");
        let result = presign_url(&config, Method::Put, "test.txt", 3600, Some(checksum));
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
