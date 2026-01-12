//! AWS Signature V4 pre-signed URL generation.
//!
//! Implements the signing algorithm for S3-compatible APIs (R2).
//! Reference: https://docs.aws.amazon.com/AmazonS3/latest/API/sigv4-query-string-auth.html

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// Pre-signed URL result
#[derive(Debug, Clone)]
pub struct PresignedUrl {
    /// The complete pre-signed URL
    pub url: String,
}

/// Errors during pre-signing
#[derive(Debug, thiserror::Error)]
pub enum PresignError {
    #[error("HMAC error")]
    Hmac,
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
/// * `content_type` - Optional content type (required for PUT)
///
/// # Returns
///
/// A `PresignedUrl` containing the URL and any required headers.
pub fn presign_url(
    config: &R2Config,
    method: Method,
    key: &str,
    expires_in_secs: u64,
    content_type: Option<&str>,
) -> Result<PresignedUrl, PresignError> {
    let now = chrono::Utc::now();
    let timestamp = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date = now.format("%Y%m%d").to_string();

    // R2 endpoint
    let host = format!("{}.r2.cloudflarestorage.com", config.account_id);

    // Region is always "auto" for R2
    let region = "auto";
    let service = "s3";

    // Credential scope
    let scope = format!("{}/{}/{}/aws4_request", date, region, service);
    let credential = format!("{}/{}", config.access_key_id, scope);

    // Signed headers
    let signed_headers = "host";

    // Build canonical query string (alphabetically sorted)
    let mut query_params = [
        ("X-Amz-Algorithm", "AWS4-HMAC-SHA256".to_string()),
        ("X-Amz-Credential", credential.clone()),
        ("X-Amz-Date", timestamp.clone()),
        ("X-Amz-Expires", expires_in_secs.to_string()),
        ("X-Amz-SignedHeaders", signed_headers.to_string()),
    ];

    // Sort by key
    query_params.sort_by(|a, b| a.0.cmp(b.0));

    let canonical_query_string = query_params
        .iter()
        .map(|(k, v)| format!("{}={}", uri_encode(k), uri_encode(v)))
        .collect::<Vec<_>>()
        .join("&");

    // Canonical headers
    let canonical_headers = format!("host:{}\n", host);

    // Canonical request
    // For pre-signed URLs, payload hash is always UNSIGNED-PAYLOAD
    // Path format for R2: /{bucket}/{key}
    let canonical_request = format!(
        "{}\n/{}/{}\n{}\n{}\n{}\nUNSIGNED-PAYLOAD",
        method.as_str(),
        uri_encode(&config.bucket),
        uri_encode_path(key),
        canonical_query_string,
        canonical_headers,
        signed_headers
    );

    // Hash the canonical request
    let canonical_request_hash = hex_sha256(canonical_request.as_bytes());

    // String to sign
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{}\n{}",
        timestamp, scope, canonical_request_hash
    );

    // Calculate signing key
    let signing_key = derive_signing_key(&config.secret_access_key, &date, region, service)?;

    // Calculate signature
    let signature = hmac_sha256_hex(&signing_key, string_to_sign.as_bytes())?;

    // Build final URL
    // URL format for R2: https://{account_id}.r2.cloudflarestorage.com/{bucket}/{key}
    let url = format!(
        "https://{}/{}/{}?{}&X-Amz-Signature={}",
        host,
        uri_encode(&config.bucket),
        uri_encode_path(key),
        canonical_query_string,
        signature
    );

    // Build required headers
    let mut headers = Vec::new();
    if let Some(ct) = content_type {
        headers.push(("Content-Type".to_string(), ct.to_string()));
    }

    Ok(PresignedUrl { url })
}

/// Derive the signing key using HMAC chain.
///
/// SigningKey = HMAC(HMAC(HMAC(HMAC("AWS4" + secret, date), region), service), "aws4_request")
fn derive_signing_key(
    secret: &str,
    date: &str,
    region: &str,
    service: &str,
) -> Result<Vec<u8>, PresignError> {
    let k_secret = format!("AWS4{}", secret);
    let k_date = hmac_sha256(k_secret.as_bytes(), date.as_bytes())?;
    let k_region = hmac_sha256(&k_date, region.as_bytes())?;
    let k_service = hmac_sha256(&k_region, service.as_bytes())?;
    let k_signing = hmac_sha256(&k_service, b"aws4_request")?;

    Ok(k_signing)
}

/// HMAC-SHA256
fn hmac_sha256(key: &[u8], data: &[u8]) -> Result<Vec<u8>, PresignError> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| PresignError::Hmac)?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

/// HMAC-SHA256 with hex output
fn hmac_sha256_hex(key: &[u8], data: &[u8]) -> Result<String, PresignError> {
    let hash = hmac_sha256(key, data)?;
    Ok(hex::encode(hash))
}

/// SHA256 hash with hex output
fn hex_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// URI encode a string (except unreserved characters)
fn uri_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                result.push(b as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", b));
            }
        }
    }
    result
}

/// URI encode a path (preserves forward slashes)
fn uri_encode_path(s: &str) -> String {
    s.split('/').map(uri_encode).collect::<Vec<_>>().join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uri_encode() {
        assert_eq!(uri_encode("test"), "test");
        assert_eq!(uri_encode("hello world"), "hello%20world");
        assert_eq!(uri_encode("a/b"), "a%2Fb");
    }

    #[test]
    fn test_uri_encode_path() {
        assert_eq!(uri_encode_path("a/b/c"), "a/b/c");
        assert_eq!(uri_encode_path("hello world/test"), "hello%20world/test");
    }

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
        // Verify bucket is in path
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

        // Key format: {space_did}/{blob_digest}
        let key = "did:key:z6MkTest/abc123def456";
        let result = presign_url(
            &config,
            Method::Put,
            key,
            3600,
            Some("application/octet-stream"),
        );
        assert!(result.is_ok());

        let url = result.unwrap();
        // Verify structure: /bucket/space_did/digest
        assert!(
            url.url
                .contains("/tonk-spaces/did%3Akey%3Az6MkTest/abc123def456")
        );
    }
}
