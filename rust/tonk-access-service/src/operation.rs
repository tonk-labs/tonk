//! Request-local storage metadata derived from a verified UCAN.
//!
//! These values are never serialized as authority, signed, cached across
//! requests, or given a new expiry. Every execution verifies the original chain.
use base64::Engine as _;
use dialog_capability::Policy;
use dialog_common::Blake3Hash;
use dialog_effects::blob::prelude::{BlobImportExt, BlobReadExt};
use dialog_effects::{archive, blob, memory};
use dialog_remote_s3::request::S3Request;
use dialog_remote_ucan::Verified;
use dialog_remote_ucan_s3::FromUcanArgs;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Put,
    Delete,
}
impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
        }
    }
}
impl std::str::FromStr for Method {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, String> {
        match value {
            "GET" => Ok(Self::Get),
            "PUT" => Ok(Self::Put),
            "DELETE" => Ok(Self::Delete),
            _ => Err("unsupported object method".into()),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Precondition {
    None,
    IfMatch(String),
    IfNoneMatch,
}

#[derive(Debug, Clone)]
pub struct Operation {
    pub method: Method,
    pub key: String,
    pub sha256: Option<Vec<u8>>,
    pub precondition: Precondition,
    pub digest: Option<Blake3Hash>,
    pub size: Option<u64>,
    pub range: Option<blob::ByteRange>,
    pub immutable: bool,
}
impl Operation {
    pub fn from_verified(verified: &Verified) -> Result<Self, String> {
        let chain = verified.chain();
        let subject = chain.subject();
        let args = chain.arguments();
        let mut digest = None;
        let mut size = None;
        let mut range = None;
        let request = match chain
            .command()
            .0
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice()
        {
            ["use", "get", "archive", "block"] | ["archive", "get"] => S3Request::from(
                &archive::Get::capability_from_args(subject, args).map_err(|e| e.to_string())?,
            ),
            ["use", "put", "archive", "block"] | ["archive", "put"] => {
                let cap =
                    archive::Put::capability_from_args(subject, args).map_err(|e| e.to_string())?;
                digest = Some(archive::PutAttenuation::of(&cap).digest.clone());
                S3Request::from(&cap)
            }
            ["use", "get", "archive", "blob"] => {
                let cap =
                    blob::Read::capability_from_args(subject, args).map_err(|e| e.to_string())?;
                range = cap.range();
                S3Request::from(&cap)
            }
            ["use", "put", "archive", "blob"] => {
                let cap =
                    blob::Import::capability_from_args(subject, args).map_err(|e| e.to_string())?;
                digest = Some(cap.digest().clone());
                size = Some(cap.size());
                S3Request::from(&cap)
            }
            ["use", "get", "memory", "cell"] | ["memory", "resolve"] => S3Request::from(
                &memory::Resolve::capability_from_args(subject, args).map_err(|e| e.to_string())?,
            ),
            ["use", "put", "memory", "cell"] | ["memory", "publish"] => S3Request::from(
                &memory::Publish::capability_from_args(subject, args).map_err(|e| e.to_string())?,
            ),
            ["use", "delete", "memory", "cell"] | ["memory", "retract"] => S3Request::from(
                &memory::Retract::capability_from_args(subject, args).map_err(|e| e.to_string())?,
            ),
            _ => return Err("unsupported UCAN operation".into()),
        };
        if request.path.starts_with("__tonk/") {
            return Err("reserved service namespace".into());
        }
        if let Some(blob::ByteRange {
            offset,
            length: Some(length),
        }) = range
        {
            if length == 0 || offset.checked_add(length).is_none() {
                return Err("invalid signed byte range".into());
            }
        }
        Ok(Self {
            method: request.method.parse()?,
            key: request.path,
            sha256: request.checksum.map(|c| c.as_bytes().to_vec()),
            precondition: match request.precondition {
                dialog_remote_s3::Precondition::None => Precondition::None,
                dialog_remote_s3::Precondition::IfMatch(v) => Precondition::IfMatch(v),
                dialog_remote_s3::Precondition::IfNoneMatch => Precondition::IfNoneMatch,
            },
            digest,
            size,
            range,
            immutable: chain.command().0.iter().any(|part| part == "archive"),
        })
    }
    pub fn range_header(&self) -> Option<String> {
        self.range.map(|r| match r.length {
            Some(n) => format!("bytes={}-{}", r.offset, r.offset + n - 1),
            None => format!("bytes={}-", r.offset),
        })
    }
}

/// A wire shape released legacy CLIs can render without --verbose.
/// Protocol support, not this advisory version, decides storage admission.
pub fn upgrade_required() -> serde_json::Value {
    serde_json::json!({
        "kind":"Unclassified",
        "detail":"This client uses an obsolete Tonk storage protocol. macOS arm64: npm install -g @tonk/cli@0.6.17 ; then run tonk --version, check PATH, and retry. Linux: a portable direct-UCAN CLI release is required; npm 0.6.17 still has the Nix-loader defect and is not a working upgrade. Release status: https://github.com/tonk-labs/tonk/pull/1085 . Do not use the frozen @stable alias. Browser users should reload the current Tonk app.",
        "code":"UpgradeRequired",
        "requiredProtocol":"ucan-direct-v1",
        "recommendedMacosArm64CliVersion":"0.6.17",
        "linuxUpgradeStatus":"portable-release-required"
    })
}

pub fn reauthorization() -> serde_json::Value {
    serde_json::json!({"kind":"ReauthorizationRequired","detail":"This object URL is retired. Retry with a current direct-UCAN client; legacy redemption explains platform-specific upgrade requirements."})
}

impl Operation {
    pub fn lift(
        permit: &dialog_remote_s3::Permit,
        address: &dialog_remote_s3::Address,
    ) -> Result<Self, String> {
        let method: Method = permit.method.parse()?;
        let path = urlencoding::decode(permit.url.path())
            .map_err(|err| format!("the authorized path is not percent-encoded: {err}"))?;
        let key = if address.path_style() {
            path.strip_prefix('/')
                .and_then(|path| path.strip_prefix(address.bucket()))
                .and_then(|path| path.strip_prefix('/'))
        } else {
            path.strip_prefix('/')
        }
        .ok_or_else(|| format!("the authorized path `{path}` is not an object in the bucket"))?
        .to_string();

        if key.starts_with("__tonk/") {
            return Err("object permits cannot address the service namespace".into());
        }
        let mut sha256 = None;
        let mut precondition = Precondition::None;
        for (name, value) in &permit.headers {
            match name.to_ascii_lowercase().as_str() {
                "x-amz-checksum-sha256" => {
                    let digest = base64::engine::general_purpose::STANDARD
                        .decode(value)
                        .map_err(|err| format!("the authorized checksum is not base64: {err}"))?;
                    if digest.len() != 32 {
                        return Err(format!(
                            "the authorized checksum is {} bytes, not 32",
                            digest.len()
                        ));
                    }
                    sha256 = Some(digest);
                }
                "if-match" => {
                    precondition = Precondition::IfMatch(value.trim_matches('"').to_string());
                }
                "if-none-match" if value == "*" => precondition = Precondition::IfNoneMatch,
                _ => {}
            }
        }

        Ok(Self {
            method,
            key,

            sha256,
            precondition,
            digest: None,
            size: None,
            range: None,
            immutable: false,
        })
    }
}
