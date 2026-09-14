//! Service-signed object permits.
//!
//! A verified invocation used to be answered with a URL presigned for
//! R2's S3 endpoint, and the client fetched the bytes from there. That
//! endpoint speaks HTTP/1.1, so a browser caps it at six connections per
//! origin, and a cold load that needs hundreds of blocks queues behind
//! that cap. The same worker that verifies the invocation holds an R2
//! binding, and its own origin is reached over HTTP/2, where the cap
//! does not exist.
//!
//! So a permit now names an object at this service — `/object/{key}` —
//! and carries what the invocation authorized as a [`Claims`] token
//! signed by the service. Whoever presents the URL gets exactly the
//! operation the chain walk admitted: the method, the key, a checksum a
//! write must match, a precondition a write or delete must satisfy, and
//! an expiry. Nothing about the request is read from anywhere else, so
//! holding a permit for one object is worth nothing against another,
//! and a permit for a read is worth nothing as a write.
//!
//! The signature is an HMAC under a key derived from the service seed.
//! The issuer and the verifier are the same service, so a shared secret
//! is the right tool: a public-key signature would let an outsider
//! verify a permit, which nobody needs, at several times the cost.
//!
//! The wire form is two query parameters: `permit`, the DAG-CBOR
//! encoding of the claims, and `signature`, the MAC over exactly those
//! bytes — both base64url without padding. Signing the encoded bytes
//! rather than a canonical rendering means there is no canonical
//! rendering to get subtly wrong on one side.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use dialog_remote_s3::{Address, Permit};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2_0_10::Sha256;
use url::Url;

/// The path every object permit is served under.
pub const PATH_PREFIX: &str = "/object/";

/// How long a permit stays valid, in seconds. The same hour the S3
/// presign carried; the client reuses a read permit for five minutes
/// of it, well clear of the edge.
pub const PERMIT_TTL: u64 = 3600;

/// The query parameter carrying the encoded [`Claims`].
const PERMIT_PARAM: &str = "permit";
/// The query parameter carrying the MAC over the encoded claims.
const SIGNATURE_PARAM: &str = "signature";

/// HKDF info for the permit MAC key. Bumping the version invalidates
/// every permit in flight, so it is a deliberate rotation rather than a
/// routine change.
const KEY_CONTEXT: &[u8] = b"tonk/permit/v1";

type HmacSha256 = Hmac<Sha256>;

/// The key permits are signed and verified with.
#[derive(Clone)]
pub struct PermitKey([u8; 32]);

impl std::fmt::Debug for PermitKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PermitKey(..)")
    }
}

impl PermitKey {
    /// Derive the permit key from the service's hex-encoded seed.
    ///
    /// Derived rather than stored, like the customer ledger keys: the
    /// service seed is the only secret, and the same seed always yields
    /// the same key, so every isolate verifies what any other issued.
    pub fn derive(seed_hex: &str) -> Result<Self, String> {
        let seed = hex::decode(seed_hex.trim())
            .map_err(|err| format!("SERVICE_SECRET_KEY is not valid hex: {err}"))?;
        let mut key = [0u8; 32];
        Hkdf::<Sha256>::new(None, &seed)
            .expand(KEY_CONTEXT, &mut key)
            .map_err(|err| format!("permit key derivation failed: {err}"))?;
        Ok(Self(key))
    }

    fn mac(&self) -> HmacSha256 {
        HmacSha256::new_from_slice(&self.0).expect("HMAC accepts a 32-byte key")
    }

    /// Sign `claims`, producing the permit the client presents.
    ///
    /// `origin` is where the permit is redeemed: the service's own
    /// origin as the client reached it, so the bytes come from the same
    /// place the authorization did.
    pub fn issue(&self, origin: &str, claims: &Claims) -> Result<Permit, String> {
        let encoded = serde_ipld_dagcbor::to_vec(claims)
            .map_err(|err| format!("permit claims did not encode: {err}"))?;
        let mut mac = self.mac();
        mac.update(&encoded);
        let signature = mac.finalize().into_bytes();

        let mut url =
            Url::parse(origin).map_err(|err| format!("permit origin is not a URL: {err}"))?;
        // `set_path`, not `join`: a key starting with `did:` would be
        // read as a scheme by a relative-reference join. It encodes
        // every character a path cannot carry except `%`, which it
        // leaves alone as a possible escape, so that one is escaped
        // here for the decode on the way back to be exact.
        url.set_path(&format!("{PATH_PREFIX}{}", claims.key.replace('%', "%25")));
        url.set_query(None);
        url.query_pairs_mut()
            .append_pair(PERMIT_PARAM, &URL_SAFE_NO_PAD.encode(&encoded))
            .append_pair(SIGNATURE_PARAM, &URL_SAFE_NO_PAD.encode(signature));

        Ok(Permit {
            url,
            method: claims.method.as_str().to_string(),
            // Everything the request needs is in the URL. Sending
            // headers as well would only cost a preflight, and a
            // browser-side header is not what the service reads anyway.
            headers: Vec::new(),
        })
    }

    /// Verify a presented permit against the request that carries it.
    ///
    /// `path` and `method` are the request's own, `query` its query
    /// string. The claims come out only once the MAC holds, the expiry
    /// has not passed, and the request is the one the claims describe.
    pub fn verify(
        &self,
        method: Method,
        path: &str,
        query: Option<&str>,
        now: u64,
    ) -> Result<Claims, PermitRefusal> {
        let (encoded, signature) = split_token(query)?;
        let mut mac = self.mac();
        mac.update(&encoded);
        mac.verify_slice(&signature)
            .map_err(|_| PermitRefusal::InvalidSignature)?;
        // Only signed bytes are decoded: a MAC failure never reaches
        // the parser, so a forged token cannot probe it.
        let claims: Claims =
            serde_ipld_dagcbor::from_slice(&encoded).map_err(|err| PermitRefusal::Malformed {
                detail: format!("permit claims did not decode: {err}"),
            })?;
        if now >= claims.expires {
            return Err(PermitRefusal::Expired {
                expires: claims.expires,
                at: now,
            });
        }
        if claims.method != method {
            return Err(PermitRefusal::Mismatch {
                detail: format!(
                    "the permit authorizes {}, not {}",
                    claims.method.as_str(),
                    method.as_str()
                ),
            });
        }
        let requested = key_from_path(path).ok_or_else(|| PermitRefusal::Mismatch {
            detail: format!("the request path is not under {PATH_PREFIX}"),
        })?;
        if requested != claims.key {
            return Err(PermitRefusal::Mismatch {
                detail: "the permit names a different object than the request path".to_string(),
            });
        }
        Ok(claims)
    }
}

/// The two token parameters, decoded.
fn split_token(query: Option<&str>) -> Result<(Vec<u8>, Vec<u8>), PermitRefusal> {
    let mut encoded = None;
    let mut signature = None;
    for (name, value) in url::form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
        match name.as_ref() {
            PERMIT_PARAM => encoded = Some(value.into_owned()),
            SIGNATURE_PARAM => signature = Some(value.into_owned()),
            _ => {}
        }
    }
    let decode = |name: &str, value: Option<String>| {
        let value = value.ok_or_else(|| PermitRefusal::Malformed {
            detail: format!("the request carries no `{name}` parameter"),
        })?;
        URL_SAFE_NO_PAD
            .decode(value)
            .map_err(|_| PermitRefusal::Malformed {
                detail: format!("the `{name}` parameter is not base64url"),
            })
    };
    Ok((
        decode(PERMIT_PARAM, encoded)?,
        decode(SIGNATURE_PARAM, signature)?,
    ))
}

/// The object key a request path under [`PATH_PREFIX`] names, or
/// `None` when the path is somewhere else.
///
/// Percent-decoded, because [`PermitKey::issue`] encodes the key into
/// the path the way any URL path is encoded, and the key the claims
/// carry is the plain one.
pub fn key_from_path(path: &str) -> Option<String> {
    let encoded = path.strip_prefix(PATH_PREFIX)?;
    urlencoding::decode(encoded)
        .ok()
        .map(|key| key.into_owned())
}

/// The operations a permit can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    Get,
    Put,
    Delete,
}

impl Method {
    /// The HTTP method name.
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

    fn from_str(method: &str) -> Result<Self, Self::Err> {
        match method {
            "GET" => Ok(Self::Get),
            "PUT" => Ok(Self::Put),
            "DELETE" => Ok(Self::Delete),
            other => Err(format!("`{other}` is not an object operation")),
        }
    }
}

/// A condition a write or delete must satisfy at the object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Precondition {
    /// Unconditional.
    None,
    /// Only if the object's current version is this one. The bare
    /// entity tag, unquoted.
    IfMatch(String),
    /// Only if the object does not exist.
    IfNoneMatch,
}

/// What a permit authorizes: exactly one operation on one object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    /// The operation.
    pub method: Method,
    /// The object key, plain.
    pub key: String,
    /// Unix seconds after which the permit is refused.
    pub expires: u64,
    /// The SHA-256 a written body must hash to. A write without one is
    /// never issued by the authorizer for content-addressed objects,
    /// but the claim is optional because a blob import declares no
    /// checksum: the blob's digest is verified by the client that
    /// streams it.
    #[serde(with = "serde_bytes", default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<Vec<u8>>,
    /// The condition a write or delete must satisfy.
    pub precondition: Precondition,
}

impl Claims {
    /// Lift the claims out of the request the authorizer described.
    ///
    /// The authorizer answers a verified invocation with a permit
    /// against an S3 [`Address`]: the URL names the object, and the
    /// headers carry the checksum and precondition it bound. That
    /// address is never handed to a client any more; this reads the
    /// operation back off the permit so it can be reissued against
    /// this service. `address` is the one the authorizer was built
    /// with, which is what says whether the bucket sits in the path.
    pub fn lift(permit: &Permit, address: &Address, expires: u64) -> Result<Self, String> {
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
            expires,
            sha256,
            precondition,
        })
    }
}

/// Why a presented permit was refused.
///
/// Answered as JSON under `kind`, the shape every refusal from this
/// service takes. The client acts on the status alone here — a 401 or
/// 403 on the object path tells it to redeem afresh — so the body is
/// for whoever is reading a log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum PermitRefusal {
    /// The token did not decode.
    Malformed { detail: String },
    /// The signature is not this service's.
    InvalidSignature,
    /// The permit has lapsed.
    Expired { expires: u64, at: u64 },
    /// The request is not the one the permit was issued for.
    Mismatch { detail: String },
}

impl PermitRefusal {
    /// The HTTP status this refusal answers with.
    pub fn status(&self) -> u16 {
        match self {
            Self::Malformed { .. } => 400,
            Self::InvalidSignature | Self::Expired { .. } => 401,
            Self::Mismatch { .. } => 403,
        }
    }
}

impl std::fmt::Display for PermitRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed { detail } => write!(f, "malformed permit: {detail}"),
            Self::InvalidSignature => f.write_str("the permit is not signed by this service"),
            Self::Expired { expires, at } => {
                write!(f, "the permit expired at {expires}, presented at {at}")
            }
            Self::Mismatch { detail } => write!(f, "permit mismatch: {detail}"),
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    const ORIGIN: &str = "https://tonk.example";

    fn key() -> PermitKey {
        PermitKey::derive(&"11".repeat(32)).unwrap()
    }

    fn claims() -> Claims {
        Claims {
            method: Method::Get,
            key: "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK/index/3vQB7B6MrGQZaxCuFg4oh"
                .to_string(),
            expires: 1_000,
            sha256: None,
            precondition: Precondition::None,
        }
    }

    /// Present `permit` back to `key` the way the handler does: the
    /// request's method, path and query.
    fn present(key: &PermitKey, permit: &Permit, now: u64) -> Result<Claims, PermitRefusal> {
        key.verify(
            permit.method.parse().unwrap(),
            permit.url.path(),
            permit.url.query(),
            now,
        )
    }

    #[dialog_common::test]
    fn it_round_trips_the_claims_through_the_url() {
        let permit = key().issue(ORIGIN, &claims()).unwrap();
        assert!(
            permit
                .url
                .as_str()
                .starts_with("https://tonk.example/object/did:key:")
        );
        assert!(permit.headers.is_empty(), "everything travels in the URL");
        assert_eq!(present(&key(), &permit, 999).unwrap(), claims());
    }

    #[dialog_common::test]
    fn it_refuses_a_lapsed_permit() {
        let permit = key().issue(ORIGIN, &claims()).unwrap();
        assert_eq!(
            present(&key(), &permit, 1_000),
            Err(PermitRefusal::Expired {
                expires: 1_000,
                at: 1_000
            })
        );
    }

    #[dialog_common::test]
    fn it_refuses_a_permit_signed_under_another_key() {
        let permit = key().issue(ORIGIN, &claims()).unwrap();
        let other = PermitKey::derive(&"22".repeat(32)).unwrap();
        assert_eq!(
            present(&other, &permit, 0),
            Err(PermitRefusal::InvalidSignature)
        );
    }

    /// The claims are what is signed: swapping in another object's
    /// claims under a real signature fails the MAC, not the key check.
    #[dialog_common::test]
    fn it_refuses_tampered_claims() {
        let signed = key().issue(ORIGIN, &claims()).unwrap();
        let forged = key()
            .issue(
                ORIGIN,
                &Claims {
                    method: Method::Put,
                    ..claims()
                },
            )
            .unwrap();
        let signature = signed
            .url
            .query_pairs()
            .find(|(name, _)| name == "signature")
            .map(|(_, value)| value.into_owned())
            .unwrap();
        let mut url = forged.url.clone();
        let permit = url
            .query_pairs()
            .find(|(name, _)| name == "permit")
            .map(|(_, value)| value.into_owned())
            .unwrap();
        url.set_query(None);
        url.query_pairs_mut()
            .append_pair("permit", &permit)
            .append_pair("signature", &signature);
        assert_eq!(
            key().verify(Method::Put, url.path(), url.query(), 0),
            Err(PermitRefusal::InvalidSignature)
        );
    }

    #[dialog_common::test]
    fn it_refuses_a_permit_used_for_another_method() {
        let permit = key().issue(ORIGIN, &claims()).unwrap();
        assert!(matches!(
            key().verify(Method::Put, permit.url.path(), permit.url.query(), 0),
            Err(PermitRefusal::Mismatch { .. })
        ));
    }

    /// A permit for one object presented at another's path is refused,
    /// even though the token itself is genuine.
    #[dialog_common::test]
    fn it_refuses_a_permit_presented_at_another_object() {
        let permit = key().issue(ORIGIN, &claims()).unwrap();
        assert!(matches!(
            key().verify(
                Method::Get,
                "/object/did:key:zOther/index/3vQB7B6MrGQZaxCuFg4oh",
                permit.url.query(),
                0
            ),
            Err(PermitRefusal::Mismatch { .. })
        ));
        assert!(matches!(
            key().verify(Method::Get, "/ucan/", permit.url.query(), 0),
            Err(PermitRefusal::Mismatch { .. })
        ));
    }

    #[dialog_common::test]
    fn it_refuses_a_request_without_a_token() {
        assert!(matches!(
            key().verify(Method::Get, "/object/x", None, 0),
            Err(PermitRefusal::Malformed { .. })
        ));
        assert!(matches!(
            key().verify(Method::Get, "/object/x", Some("permit=***&signature=!"), 0),
            Err(PermitRefusal::Malformed { .. })
        ));
    }

    /// Keys with characters a URL path encodes survive the trip: the
    /// request path arrives encoded and is decoded before comparison.
    #[dialog_common::test]
    fn it_keeps_a_key_with_awkward_characters_intact() {
        let awkward = Claims {
            key: "did:key:zSubject/spa ce/cell?name#frag%20é".to_string(),
            ..claims()
        };
        let permit = key().issue(ORIGIN, &awkward).unwrap();
        assert_eq!(present(&key(), &permit, 0).unwrap().key, awkward.key);
    }

    #[dialog_common::test]
    fn it_keeps_the_origin_port() {
        let permit = key().issue("http://127.0.0.1:8090", &claims()).unwrap();
        assert!(
            permit
                .url
                .as_str()
                .starts_with("http://127.0.0.1:8090/object/")
        );
    }

    fn address(path_style: bool) -> Address {
        Address::builder("https://s3.example")
            .region("auto")
            .bucket("spaces")
            .path_style(path_style)
            .build()
            .unwrap()
    }

    #[dialog_common::test]
    fn it_lifts_a_read_off_a_virtual_hosted_permit() {
        let permit = Permit {
            url: "https://spaces.s3.example/did:key:zSubject/index/abc"
                .parse()
                .unwrap(),
            method: "GET".to_string(),
            headers: vec![("host".to_string(), "spaces.s3.example".to_string())],
        };
        let lifted = Claims::lift(&permit, &address(false), 77).unwrap();
        assert_eq!(
            lifted,
            Claims {
                method: Method::Get,
                key: "did:key:zSubject/index/abc".to_string(),
                expires: 77,
                sha256: None,
                precondition: Precondition::None,
            }
        );
    }

    #[dialog_common::test]
    fn it_lifts_a_conditional_write_off_a_path_style_permit() {
        let digest = [7u8; 32];
        let permit = Permit {
            url: "http://127.0.0.1:9000/spaces/did:key:zSubject/space/main"
                .parse()
                .unwrap(),
            method: "PUT".to_string(),
            headers: vec![
                ("host".to_string(), "127.0.0.1:9000".to_string()),
                (
                    "x-amz-checksum-sha256".to_string(),
                    base64::engine::general_purpose::STANDARD.encode(digest),
                ),
                ("if-match".to_string(), "\"abcdef\"".to_string()),
            ],
        };
        let lifted = Claims::lift(&permit, &address(true), 1).unwrap();
        assert_eq!(lifted.method, Method::Put);
        assert_eq!(lifted.key, "did:key:zSubject/space/main");
        assert_eq!(lifted.sha256, Some(digest.to_vec()));
        assert_eq!(
            lifted.precondition,
            Precondition::IfMatch("abcdef".to_string())
        );
    }

    #[dialog_common::test]
    fn it_lifts_a_create_only_write() {
        let permit = Permit {
            url: "https://spaces.s3.example/did:key:zSubject/space/main"
                .parse()
                .unwrap(),
            method: "PUT".to_string(),
            headers: vec![("if-none-match".to_string(), "*".to_string())],
        };
        let lifted = Claims::lift(&permit, &address(false), 1).unwrap();
        assert_eq!(lifted.precondition, Precondition::IfNoneMatch);
    }

    #[dialog_common::test]
    fn it_refuses_to_lift_an_operation_it_cannot_serve() {
        let permit = Permit {
            url: "https://spaces.s3.example/did:key:zSubject/index/abc"
                .parse()
                .unwrap(),
            method: "POST".to_string(),
            headers: vec![],
        };
        assert!(Claims::lift(&permit, &address(false), 1).is_err());
    }

    #[dialog_common::test]
    fn it_answers_each_refusal_with_its_status() {
        assert_eq!(
            PermitRefusal::Malformed {
                detail: String::new()
            }
            .status(),
            400
        );
        assert_eq!(PermitRefusal::InvalidSignature.status(), 401);
        assert_eq!(PermitRefusal::Expired { expires: 1, at: 2 }.status(), 401);
        assert_eq!(
            PermitRefusal::Mismatch {
                detail: String::new()
            }
            .status(),
            403
        );
        let json = serde_json::to_value(PermitRefusal::InvalidSignature).unwrap();
        assert_eq!(json["kind"], "InvalidSignature");
    }
}
