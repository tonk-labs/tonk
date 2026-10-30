//! Who is asking: OAuth clients, by metadata document or registration.
//!
//! Every client is public. None holds a secret, and none needs one: the
//! user approves each authorization in the browser, and PKCE binds the
//! code to whoever started it. A client identity is therefore only
//! redirect policy -- where a code may be delivered -- plus a name to
//! show the user. Two ways to have one:
//!
//! - **Client ID Metadata Document.** The `client_id` is an HTTPS URL,
//!   and the document there names the client and its callbacks. The
//!   host is the client's identity, so the approval page can say who it
//!   is.
//! - **Dynamic registration** (RFC 7591). The client posts its metadata
//!   and gets back a `client_id` that *is* that metadata, signed by this
//!   service. Nothing is stored: an unauthenticated endpoint that wrote
//!   a row per request would be a way to fill the database, and a signed
//!   identifier answers the only question ever asked of it (which
//!   callbacks, which name) without one. Self-asserted, so the approval
//!   page should treat its name as a claim.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2_0_10::Sha256;
use url::{Host, Url};

/// Longest redirect URI accepted, in bytes.
pub const MAX_REDIRECT_URI: usize = 2048;

/// Most redirect URIs one client may name.
pub const MAX_REDIRECT_URIS: usize = 8;

/// Longest client name kept, in characters. Longer names are cut.
pub const MAX_CLIENT_NAME: usize = 128;

/// Longest registered `client_id`, in bytes. It travels in every
/// `/authorize` URL, so a registration carrying more metadata than fits
/// is refused rather than minted.
pub const MAX_REGISTERED_ID: usize = 2048;

/// Largest client metadata document fetched, in bytes.
pub const MAX_DOCUMENT: usize = 16 * 1024;

/// HKDF context for the key registered client IDs are signed under.
const KEY_CONTEXT: &[u8] = b"tonk/oauth/client/v1";

type HmacSha256 = Hmac<Sha256>;

/// How a client is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Registration {
    /// Its `client_id` is the URL of a metadata document.
    MetadataDocument,
    /// It registered here, and its `client_id` carries its metadata.
    Dynamic,
}

impl Registration {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MetadataDocument => "metadata-document",
            Self::Dynamic => "dynamic",
        }
    }
}

/// A client that may be sent through `/authorize`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Client {
    /// The identifier the client presents.
    pub id: String,
    /// What it calls itself.
    pub name: String,
    /// Where a code for it may be delivered.
    pub redirect_uris: Vec<String>,
    /// How it is known.
    pub registration: Registration,
}

impl Client {
    /// Whether a code may be delivered to `uri`.
    ///
    /// Exact match, except that a loopback callback matches on any port
    /// (RFC 8252 §7.3): a native client listens wherever the OS gives it
    /// a port, so a document cannot name one in advance.
    pub fn accepts(&self, uri: &str) -> bool {
        let Ok(presented) = redirect_uri(uri) else {
            return false;
        };
        self.redirect_uris.iter().any(|registered| {
            if registered == uri {
                return true;
            }
            let Ok(registered) = Url::parse(registered) else {
                return false;
            };
            is_loopback(&registered)
                && is_loopback(&presented)
                && without_port(&registered) == without_port(&presented)
        })
    }
}

/// Why a client could not be identified.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientError {
    /// The `client_id` names nothing this service can use.
    #[error("{0}")]
    Unknown(String),
    /// The metadata (registered or fetched) is not acceptable.
    #[error("{0}")]
    InvalidMetadata(String),
}

/// Validate a redirect URI on its own: HTTPS on a public DNS name, or
/// plain HTTP on a loopback address.
///
/// It must also be what it parses to, so nothing about it changes
/// between here and the browser following it.
pub fn redirect_uri(uri: &str) -> Result<Url, String> {
    if uri.len() > MAX_REDIRECT_URI {
        return Err(format!(
            "redirect_uri is over the {MAX_REDIRECT_URI}-byte limit"
        ));
    }
    let url = Url::parse(uri).map_err(|e| format!("redirect_uri is not a URL: {e}"))?;
    if url.as_str() != uri {
        return Err(format!(
            "redirect_uri must be written as it parses: {}",
            url.as_str()
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("redirect_uri must not carry credentials".into());
    }
    if url.fragment().is_some() {
        return Err("redirect_uri must not carry a fragment".into());
    }
    match url.scheme() {
        "https" if is_public_name(&url) => Ok(url),
        "https" => Err("an https redirect_uri must name a public DNS host".into()),
        "http" if is_loopback(&url) => Ok(url),
        "http" => Err("plain http is accepted only for localhost callbacks".into()),
        scheme => Err(format!("redirect_uri scheme {scheme} is not supported")),
    }
}

/// `localhost`, `127.0.0.1` or `[::1]`, over plain HTTP.
fn is_loopback(url: &Url) -> bool {
    url.scheme() == "http"
        && match url.host() {
            Some(Host::Domain(name)) => name == "localhost",
            Some(Host::Ipv4(address)) => address.is_loopback(),
            Some(Host::Ipv6(address)) => address.is_loopback(),
            None => false,
        }
}

/// A DNS name that could be someone's on the public internet: dotted,
/// not an address, and not one of the names reserved for private
/// networks. No lookup is made; this only refuses the obvious.
fn is_public_name(url: &Url) -> bool {
    let Some(Host::Domain(name)) = url.host() else {
        return false;
    };
    const PRIVATE: [&str; 5] = [".localhost", ".local", ".internal", ".home.arpa", ".lan"];
    name.contains('.')
        && !name.ends_with('.')
        && name != "localhost"
        && !PRIVATE.iter().any(|suffix| name.ends_with(suffix))
}

fn without_port(url: &Url) -> Url {
    let mut url = url.clone();
    let _ = url.set_port(None);
    url
}

/// A client name fit to show: trimmed, without control characters, and
/// cut to [`MAX_CLIENT_NAME`]. Falls back to `fallback` when nothing is
/// left.
fn display_name(name: Option<&str>, fallback: &str) -> String {
    let name: String = name
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(MAX_CLIENT_NAME)
        .collect();
    if name.is_empty() {
        fallback.to_owned()
    } else {
        name
    }
}

/// The host a callback goes to, for naming a client that named nothing.
fn host_of(uri: &str) -> String {
    Url::parse(uri)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| uri.to_owned())
}

/// Validate a list of redirect URIs from metadata.
fn redirect_uris(uris: Option<Vec<String>>) -> Result<Vec<String>, ClientError> {
    let uris = uris.unwrap_or_default();
    if uris.is_empty() {
        return Err(ClientError::InvalidMetadata(
            "redirect_uris must name at least one callback".into(),
        ));
    }
    if uris.len() > MAX_REDIRECT_URIS {
        return Err(ClientError::InvalidMetadata(format!(
            "redirect_uris names more than {MAX_REDIRECT_URIS} callbacks"
        )));
    }
    for uri in &uris {
        redirect_uri(uri).map_err(ClientError::InvalidMetadata)?;
    }
    Ok(uris)
}

/// Only public clients: no token endpoint authentication, so nothing for
/// a client to keep secret.
fn public_client(method: Option<&str>) -> Result<(), ClientError> {
    match method {
        None | Some("none") => Ok(()),
        Some(other) => Err(ClientError::InvalidMetadata(format!(
            "token_endpoint_auth_method {other} is not supported; only none is"
        ))),
    }
}

/// Client metadata, as a registration or a metadata document carries
/// it. Fields this service does not use are ignored, as RFC 7591 asks.
#[derive(Debug, Deserialize)]
pub struct Metadata {
    client_id: Option<String>,
    client_name: Option<String>,
    redirect_uris: Option<Vec<String>>,
    token_endpoint_auth_method: Option<String>,
    grant_types: Option<Vec<String>>,
    response_types: Option<Vec<String>>,
}

impl Metadata {
    /// The grant and response types, when given, must include the one
    /// flow this server runs. Others (`refresh_token`, most often) are
    /// dropped rather than refused: the answer says what was granted.
    fn runs_authorization_code(&self) -> Result<(), ClientError> {
        if let Some(grants) = &self.grant_types
            && !grants.iter().any(|grant| grant == "authorization_code")
        {
            return Err(ClientError::InvalidMetadata(
                "grant_types must include authorization_code".into(),
            ));
        }
        if let Some(responses) = &self.response_types
            && !responses.iter().any(|response| response == "code")
        {
            return Err(ClientError::InvalidMetadata(
                "response_types must include code".into(),
            ));
        }
        Ok(())
    }
}

/// What a registered `client_id` carries, signed.
#[derive(Debug, Serialize, Deserialize)]
struct Registered {
    name: String,
    redirect_uris: Vec<String>,
}

/// The key registered client IDs are signed under, derived from the
/// service seed.
#[derive(Clone)]
pub struct ClientKey([u8; 32]);

impl std::fmt::Debug for ClientKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClientKey(..)")
    }
}

impl ClientKey {
    /// Derive the key from the service's hex seed.
    pub fn derive(seed_hex: &str) -> Result<Self, String> {
        let seed = hex::decode(seed_hex).map_err(|e| format!("service seed is not hex: {e}"))?;
        let mut key = [0u8; 32];
        Hkdf::<Sha256>::new(None, &seed)
            .expand(KEY_CONTEXT, &mut key)
            .map_err(|e| format!("client key derivation failed: {e}"))?;
        Ok(Self(key))
    }

    fn mac(&self) -> HmacSha256 {
        HmacSha256::new_from_slice(&self.0).expect("HMAC accepts a 32-byte key")
    }

    /// Register a client (RFC 7591): validate its metadata and mint the
    /// `client_id` that carries it.
    pub fn register(&self, body: &[u8]) -> Result<Client, ClientError> {
        let metadata: Metadata = serde_json::from_slice(body)
            .map_err(|e| ClientError::InvalidMetadata(format!("not client metadata: {e}")))?;
        public_client(metadata.token_endpoint_auth_method.as_deref())?;
        metadata.runs_authorization_code()?;
        let redirect_uris = redirect_uris(metadata.redirect_uris)?;
        let name = display_name(metadata.client_name.as_deref(), &host_of(&redirect_uris[0]));
        let registered = Registered {
            name,
            redirect_uris,
        };
        let payload = serde_json::to_vec(&registered).expect("registration serializes");
        let mut mac = self.mac();
        mac.update(&payload);
        let id = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(&payload),
            URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
        );
        if id.len() > MAX_REGISTERED_ID {
            return Err(ClientError::InvalidMetadata(format!(
                "the registration is over the {MAX_REGISTERED_ID}-byte client_id limit"
            )));
        }
        Ok(Client {
            id,
            name: registered.name,
            redirect_uris: registered.redirect_uris,
            registration: Registration::Dynamic,
        })
    }

    /// The client a registered `client_id` carries, when this service
    /// signed it.
    pub fn registered(&self, id: &str) -> Option<Client> {
        let (payload, signature) = id.split_once('.')?;
        let payload = URL_SAFE_NO_PAD.decode(payload).ok()?;
        let signature = URL_SAFE_NO_PAD.decode(signature).ok()?;
        let mut mac = self.mac();
        mac.update(&payload);
        mac.verify_slice(&signature).ok()?;
        let registered: Registered = serde_json::from_slice(&payload).ok()?;
        Some(Client {
            id: id.to_owned(),
            name: registered.name,
            redirect_uris: registered.redirect_uris,
            registration: Registration::Dynamic,
        })
    }
}

/// Fetches client metadata documents. The worker fetches over the
/// network; tests answer from memory.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait Documents {
    /// The body at `url`, no more than [`MAX_DOCUMENT`] bytes of it.
    /// Redirects are not followed: the document must be at its own URL.
    async fn fetch(&self, url: &Url) -> Result<Vec<u8>, String>;
}

/// Validate a `client_id` as a metadata document URL: HTTPS on a public
/// name, a path, and nothing that would change it in parsing.
pub fn document_url(id: &str) -> Result<Url, ClientError> {
    let url = Url::parse(id).map_err(|_| ClientError::Unknown("client_id is unknown".into()))?;
    let unacceptable = url.as_str() != id
        || url.scheme() != "https"
        || !is_public_name(&url)
        || url.path() == "/"
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some();
    if unacceptable {
        return Err(ClientError::Unknown(
            "a client_id URL must be https on a public host, with a path and no fragment".into(),
        ));
    }
    Ok(url)
}

/// Read a fetched metadata document as the client at `url`.
pub fn from_document(url: &Url, body: &[u8]) -> Result<Client, ClientError> {
    if body.len() > MAX_DOCUMENT {
        return Err(ClientError::InvalidMetadata(format!(
            "the client metadata document is over {MAX_DOCUMENT} bytes"
        )));
    }
    let metadata: Metadata = serde_json::from_slice(body).map_err(|e| {
        ClientError::InvalidMetadata(format!("the client metadata document is not metadata: {e}"))
    })?;
    if metadata.client_id.as_deref() != Some(url.as_str()) {
        return Err(ClientError::InvalidMetadata(
            "the client metadata document names a different client_id".into(),
        ));
    }
    public_client(metadata.token_endpoint_auth_method.as_deref())?;
    metadata.runs_authorization_code()?;
    let redirect_uris = redirect_uris(metadata.redirect_uris)?;
    let host = url.host_str().unwrap_or_default();
    Ok(Client {
        id: url.as_str().to_owned(),
        name: display_name(metadata.client_name.as_deref(), host),
        redirect_uris,
        registration: Registration::MetadataDocument,
    })
}

/// The client `id` names: a metadata document fetched from it, or a
/// registration this service signed.
pub async fn resolve<D: Documents + ?Sized>(
    id: &str,
    key: &ClientKey,
    documents: &D,
) -> Result<Client, ClientError> {
    if id.starts_with("https://") {
        let url = document_url(id)?;
        let body = documents.fetch(&url).await.map_err(|e| {
            ClientError::Unknown(format!(
                "the client metadata document could not be fetched: {e}"
            ))
        })?;
        return from_document(&url, &body);
    }
    key.registered(id)
        .ok_or_else(|| ClientError::Unknown("client_id is unknown".into()))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn key() -> ClientKey {
        ClientKey::derive(&"11".repeat(32)).unwrap()
    }

    #[dialog_common::test]
    fn it_accepts_https_and_loopback_callbacks() {
        for uri in [
            "https://claude.ai/api/mcp/auth_callback",
            "http://localhost:6274/oauth/callback",
            "http://127.0.0.1:33418/",
            "http://[::1]:8080/callback",
        ] {
            assert!(redirect_uri(uri).is_ok(), "{uri} is refused");
        }
    }

    #[dialog_common::test]
    fn it_refuses_callbacks_that_could_leak_a_code() {
        for uri in [
            "http://example.com/callback",
            "https://10.0.0.1/callback",
            "https://intranet/callback",
            "https://printer.local/callback",
            "https://user:pass@example.com/callback",
            "https://example.com/callback#fragment",
            "javascript:alert(1)",
            // Not as it parses: the browser would follow something else.
            "https://EXAMPLE.com/callback",
            "http://127.1/callback",
        ] {
            assert!(redirect_uri(uri).is_err(), "{uri} is accepted");
        }
    }

    #[dialog_common::test]
    fn it_matches_a_loopback_callback_on_any_port() {
        let client = Client {
            id: "x".into(),
            name: "x".into(),
            redirect_uris: vec![
                "http://localhost/callback".into(),
                "https://example.com/callback".into(),
            ],
            registration: Registration::Dynamic,
        };
        assert!(client.accepts("http://localhost:54321/callback"));
        assert!(client.accepts("https://example.com/callback"));
        assert!(!client.accepts("https://example.com:8443/callback"));
        assert!(!client.accepts("http://localhost:54321/other"));
        assert!(!client.accepts("http://127.0.0.1:54321/callback"));
    }

    #[dialog_common::test]
    fn it_registers_a_client_into_its_own_identifier() {
        let client = key()
            .register(
                br#"{"client_name":"Inspector","redirect_uris":["http://localhost:6274/oauth/callback"],
                    "grant_types":["authorization_code","refresh_token"],
                    "token_endpoint_auth_method":"none","logo_uri":"https://example.com/logo.png"}"#,
            )
            .unwrap();
        assert_eq!(client.name, "Inspector");
        assert_eq!(key().registered(&client.id), Some(client.clone()));

        let other = ClientKey::derive(&"22".repeat(32)).unwrap();
        assert_eq!(other.registered(&client.id), None, "another key's id");
        let (payload, _) = client.id.split_once('.').unwrap();
        let forged = format!("{payload}.{}", URL_SAFE_NO_PAD.encode([0u8; 32]));
        assert_eq!(key().registered(&forged), None, "an unsigned id");
    }

    #[dialog_common::test]
    fn it_refuses_registrations_it_cannot_serve() {
        for body in [
            r#"{"redirect_uris":[]}"#,
            r#"{"redirect_uris":["http://example.com/cb"]}"#,
            r#"{"redirect_uris":["https://example.com/cb"],"token_endpoint_auth_method":"client_secret_basic"}"#,
            r#"{"redirect_uris":["https://example.com/cb"],"grant_types":["client_credentials"]}"#,
            r#"{"redirect_uris":["https://example.com/cb"],"response_types":["token"]}"#,
            r#"[]"#,
        ] {
            assert!(
                matches!(
                    key().register(body.as_bytes()),
                    Err(ClientError::InvalidMetadata(_))
                ),
                "{body} registers"
            );
        }
    }

    #[dialog_common::test]
    fn it_names_an_unnamed_client_by_its_callback() {
        let client = key()
            .register(
                br#"{"client_name":"  \u0007 ","redirect_uris":["https://app.example.com/cb"]}"#,
            )
            .unwrap();
        assert_eq!(client.name, "app.example.com");
    }

    #[dialog_common::test]
    fn it_reads_a_metadata_document_at_its_own_url() {
        let url = document_url("https://claude.ai/oauth/claude-code-client-metadata").unwrap();
        let client = from_document(
            &url,
            br#"{"client_id":"https://claude.ai/oauth/claude-code-client-metadata",
                "client_name":"Claude Code","redirect_uris":["http://localhost/callback"]}"#,
        )
        .unwrap();
        assert_eq!(client.name, "Claude Code");
        assert_eq!(client.registration, Registration::MetadataDocument);
        assert!(client.accepts("http://localhost:3118/callback"));

        assert!(
            from_document(
                &url,
                br#"{"client_id":"https://evil.example/doc","redirect_uris":["http://localhost/callback"]}"#,
            )
            .is_err(),
            "a document for another client_id"
        );
    }

    #[dialog_common::test]
    fn it_refuses_client_id_urls_that_are_not_documents() {
        for id in [
            "http://example.com/client",
            "https://example.com/",
            "https://127.0.0.1/client",
            "https://example.com/client#x",
            "https://example.com/a/../client",
        ] {
            assert!(document_url(id).is_err(), "{id} is accepted");
        }
    }
}
