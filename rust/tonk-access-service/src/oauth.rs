//! OAuth authorization server for `/mcp`.
//!
//! The provider origin is both the resource (`<origin>/mcp`) and the
//! authorization server, so an LLM host discovers one from the other and
//! the user approves on the origin their passkey works on. The server
//! itself holds no keys and mints nothing: the approval page does all
//! key work in the browser and hands back a finished token. The token is
//! opaque here.
//!
//! ```text
//! GET  /.well-known/oauth-protected-resource/mcp   resource metadata (RFC 9728)
//! GET  /.well-known/oauth-authorization-server     server metadata (RFC 8414)
//! POST /register                                   dynamic registration (RFC 7591)
//! ```
//!
//! This module is the platform-independent core, shared by the Worker
//! handler and the native server in `helpers`; reading bodies and
//! writing responses stays with them.

pub mod client;

use serde_json::json;

use client::ClientKey;

/// The resource this server issues tokens for, under the origin.
pub const RESOURCE_PATH: &str = "/mcp";

/// Protected resource metadata for [`RESOURCE_PATH`].
pub const PROTECTED_RESOURCE_PATH: &str = "/.well-known/oauth-protected-resource/mcp";

/// Authorization server metadata.
pub const AUTHORIZATION_SERVER_PATH: &str = "/.well-known/oauth-authorization-server";

/// The authorization endpoint.
pub const AUTHORIZE_PATH: &str = "/authorize";

/// The token endpoint.
pub const TOKEN_PATH: &str = "/token";

/// The dynamic registration endpoint.
pub const REGISTER_PATH: &str = "/register";

/// The largest body any endpoint here reads, in bytes. Callers bound
/// the read with it before handing the body over.
pub const MAX_BODY: usize = 64 * 1024;

/// An endpoint of this server, by path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// Protected resource metadata.
    ProtectedResource,
    /// Authorization server metadata.
    AuthorizationServer,
    /// Dynamic client registration.
    Register,
}

impl Endpoint {
    /// The endpoint at `path`, if it is one of these.
    pub fn of(path: &str) -> Option<Self> {
        match path {
            PROTECTED_RESOURCE_PATH => Some(Self::ProtectedResource),
            AUTHORIZATION_SERVER_PATH => Some(Self::AuthorizationServer),
            REGISTER_PATH => Some(Self::Register),
            _ => None,
        }
    }

    /// Whether other origins may call it. The metadata and registration
    /// endpoints are called by clients that may run in a browser (MCP
    /// Inspector does).
    fn cross_origin(&self) -> bool {
        matches!(
            self,
            Self::ProtectedResource | Self::AuthorizationServer | Self::Register
        )
    }
}

/// The parts of a request this server reads.
#[derive(Debug, Default, Clone, Copy)]
pub struct Request<'a> {
    /// The HTTP method, upper case.
    pub method: &'a str,
    /// The query string, without its `?`.
    pub query: Option<&'a str>,
    /// `Content-Type`.
    pub content_type: Option<&'a str>,
    /// `Origin`.
    pub origin: Option<&'a str>,
    /// `Cookie`.
    pub cookie: Option<&'a str>,
    /// The body, already bounded by [`MAX_BODY`].
    pub body: &'a [u8],
}

/// A response, for the host to write out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

impl Answer {
    fn json(status: u16, value: serde_json::Value) -> Self {
        Self {
            status,
            headers: vec![
                ("Content-Type", "application/json".into()),
                ("Cache-Control", "no-store".into()),
            ],
            body: serde_json::to_vec(&value).expect("JSON serializes"),
        }
    }

    /// An OAuth error response (RFC 6749 §5.2).
    fn error(status: u16, error: &str, description: impl Into<String>) -> Self {
        Self::json(
            status,
            json!({ "error": error, "error_description": description.into() }),
        )
    }

    fn with(mut self, name: &'static str, value: String) -> Self {
        self.headers.push((name, value));
        self
    }

    fn cors(self) -> Self {
        self.with("Access-Control-Allow-Origin", "*".into())
            .with("Access-Control-Allow-Methods", "GET, POST, OPTIONS".into())
            .with(
                "Access-Control-Allow-Headers",
                "Content-Type, Authorization, MCP-Protocol-Version".into(),
            )
    }
}

/// The authorization server, for one request.
pub struct AuthorizationServer<'a> {
    /// What registered client IDs are signed under.
    pub key: &'a ClientKey,
    /// This origin, as clients reach it: `https://tonk.foundation`.
    pub origin: &'a str,
    /// The clock, in unix seconds.
    pub now: u64,
}

impl AuthorizationServer<'_> {
    /// The resource tokens are issued for.
    pub fn resource(&self) -> String {
        format!("{}{RESOURCE_PATH}", self.origin)
    }

    /// Answer `request` at `endpoint`.
    pub async fn handle(&self, endpoint: &Endpoint, request: &Request<'_>) -> Answer {
        let cross_origin = endpoint.cross_origin();
        let answer = match (request.method, endpoint) {
            ("OPTIONS", _) if cross_origin => Answer {
                status: 204,
                headers: vec![("Access-Control-Max-Age", crate::PREFLIGHT_MAX_AGE.into())],
                body: Vec::new(),
            },
            ("GET", Endpoint::ProtectedResource) => self.protected_resource(),
            ("GET", Endpoint::AuthorizationServer) => self.authorization_server(),
            ("POST", Endpoint::Register) => self.register(request),
            _ => Answer::error(405, "invalid_request", "method not allowed"),
        };
        if cross_origin { answer.cors() } else { answer }
    }

    /// RFC 9728: the resource, and that this origin authorizes it.
    fn protected_resource(&self) -> Answer {
        Answer::json(
            200,
            json!({
                "resource": self.resource(),
                "authorization_servers": [self.origin],
                "bearer_methods_supported": ["header"],
                "resource_name": "Tonk",
            }),
        )
    }

    /// RFC 8414. Public clients only, S256 only, codes only.
    fn authorization_server(&self) -> Answer {
        let origin = self.origin;
        Answer::json(
            200,
            json!({
                "issuer": origin,
                "authorization_endpoint": format!("{origin}{AUTHORIZE_PATH}"),
                "token_endpoint": format!("{origin}{TOKEN_PATH}"),
                "registration_endpoint": format!("{origin}{REGISTER_PATH}"),
                "response_types_supported": ["code"],
                "response_modes_supported": ["query"],
                "grant_types_supported": ["authorization_code"],
                "token_endpoint_auth_methods_supported": ["none"],
                "code_challenge_methods_supported": ["S256"],
                "client_id_metadata_document_supported": true,
                "authorization_response_iss_parameter_supported": true,
            }),
        )
    }

    /// RFC 7591. The `client_id` answered carries the metadata; see
    /// [`client`].
    fn register(&self, request: &Request<'_>) -> Answer {
        if !media_type_is(request.content_type, "application/json") {
            return Answer::error(
                400,
                "invalid_client_metadata",
                "registration is a JSON body",
            );
        }
        match self.key.register(request.body) {
            Ok(client) => Answer::json(
                201,
                json!({
                    "client_id": client.id,
                    "client_id_issued_at": self.now,
                    "client_name": client.name,
                    "redirect_uris": client.redirect_uris,
                    "grant_types": ["authorization_code"],
                    "response_types": ["code"],
                    "token_endpoint_auth_method": "none",
                }),
            ),
            Err(error) => Answer::error(400, "invalid_client_metadata", error.to_string()),
        }
    }
}

/// Whether `content_type` names `expected`, parameters aside.
fn media_type_is(content_type: Option<&str>, expected: &str) -> bool {
    content_type
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case(expected))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
