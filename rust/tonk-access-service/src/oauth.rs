//! OAuth authorization server for `/mcp`.
//!
//! The provider origin is both the resource (`<origin>/mcp`) and the
//! authorization server, so an LLM host discovers one from the other and
//! the user approves on the origin their passkey works on. The server
//! itself holds no keys and mints nothing: the approval page does all
//! key work in the browser and hands back a finished token, and this
//! server only stores it against a code until `/token` collects it. The
//! token is opaque here.
//!
//! ```text
//! GET  /.well-known/oauth-protected-resource/mcp   resource metadata (RFC 9728)
//! GET  /.well-known/oauth-authorization-server     server metadata (RFC 8414)
//! POST /register                                   dynamic registration (RFC 7591)
//! GET  /authorize                                  validate, then hand off to the page
//! GET  /authorize/{request}                        what the page shows
//! POST /authorize/{request}                        the page's answer: a token, or a refusal
//! POST /token                                      code + verifier for the token, once
//! ```
//!
//! `/authorize` checks everything a client controls -- who it is, where
//! the code goes, PKCE, the resource -- and records the request. It then
//! sends the browser to [`APPROVAL_PAGE`] with a handle naming the
//! request, and sets a cookie binding the request to that browser. The
//! page reads `GET /authorize/{request}` for the client's name and
//! callback, and when the user is done posts the token (or a refusal)
//! back to the same path from the same origin, cookie attached. The
//! answer is where to send the browser: the callback, with a code.
//!
//! A code lives [`CODE_TTL`] seconds and redeems once. `/token` checks
//! the verifier, the callback, the client and the resource in the same
//! statement that removes the code, so racing redemptions cannot both
//! win. The token response carries no `expires_in` and no
//! `refresh_token`: a session is valid until revoked, and renewing one
//! would need the user's browser anyway.
//!
//! This module is the platform-independent core, shared by the Worker
//! handler and the native server in `helpers`; reading bodies and
//! writing responses stays with them.

pub mod client;

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hkdf::Hkdf;
use serde::Deserialize;
use serde_json::json;
use sha2_0_10::{Digest, Sha256};
use url::Url;

use crate::store::oauth::{AuthorizationStore, PendingAuthorization, Redemption};
use client::{ClientError, ClientKey, Documents};

/// The resource this server issues tokens for, under the origin.
pub const RESOURCE_PATH: &str = "/mcp";

/// Protected resource metadata for [`RESOURCE_PATH`].
pub const PROTECTED_RESOURCE_PATH: &str = "/.well-known/oauth-protected-resource/mcp";

/// Authorization server metadata.
pub const AUTHORIZATION_SERVER_PATH: &str = "/.well-known/oauth-authorization-server";

/// The authorization endpoint, and the prefix of the page's request
/// endpoint under it.
pub const AUTHORIZE_PATH: &str = "/authorize";

/// The token endpoint.
pub const TOKEN_PATH: &str = "/token";

/// The dynamic registration endpoint.
pub const REGISTER_PATH: &str = "/register";

/// The app route that approves an authorization. `/authorize` sends the
/// browser here with `?request=<handle>`.
pub const APPROVAL_PAGE: &str = "/settings/agent";

/// How long the approval page has, from `/authorize`, in seconds: long
/// enough to sign in and choose spaces.
pub const REQUEST_TTL: u64 = 10 * 60;

/// How long a code lives once the page answers, in seconds.
pub const CODE_TTL: u64 = 60;

/// How much longer than its request the browser keeps a request's cookie.
const COOKIE_GRACE: u64 = 60 * 60;

/// The largest body any endpoint here reads, in bytes. Callers bound
/// the read with it before handing the body over.
pub const MAX_BODY: usize = 64 * 1024;

/// The longest `state` carried through.
const MAX_STATE: usize = 2048;

/// The longest `/authorize` query read.
const MAX_QUERY: usize = 8192;

/// The longest token the page may post.
const MAX_TOKEN: usize = 32 * 1024;

/// HKDF context for the key a token is sealed under, from its code.
const SEAL_CONTEXT: &[u8] = b"tonk/oauth/code/v1";

/// An endpoint of this server, by path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// Protected resource metadata.
    ProtectedResource,
    /// Authorization server metadata.
    AuthorizationServer,
    /// Dynamic client registration.
    Register,
    /// The authorization endpoint.
    Authorize,
    /// One pending request, as the approval page reads and answers it.
    Request(String),
    /// The token endpoint.
    Token,
}

impl Endpoint {
    /// The endpoint at `path`, if it is one of these.
    pub fn of(path: &str) -> Option<Self> {
        match path {
            PROTECTED_RESOURCE_PATH => Some(Self::ProtectedResource),
            AUTHORIZATION_SERVER_PATH => Some(Self::AuthorizationServer),
            REGISTER_PATH => Some(Self::Register),
            AUTHORIZE_PATH => Some(Self::Authorize),
            TOKEN_PATH => Some(Self::Token),
            _ => path
                .strip_prefix(AUTHORIZE_PATH)
                .and_then(|rest| rest.strip_prefix('/'))
                .filter(|handle| !handle.is_empty() && !handle.contains('/'))
                .map(|handle| Self::Request(handle.to_owned())),
        }
    }

    /// Whether other origins may call it. The metadata, registration and
    /// token endpoints are called by clients that may run in a browser
    /// (MCP Inspector does); the request endpoint is the approval page's
    /// alone, and `/authorize` is a navigation.
    fn cross_origin(&self) -> bool {
        matches!(
            self,
            Self::ProtectedResource | Self::AuthorizationServer | Self::Register | Self::Token
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

    /// An error shown to the user in the browser, where redirecting to
    /// the callback is not safe: the client or callback is in doubt.
    fn page(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            headers: vec![
                ("Content-Type", "text/plain; charset=utf-8".into()),
                ("X-Content-Type-Options", "nosniff".into()),
                ("Cache-Control", "no-store".into()),
            ],
            body: format!("Tonk could not start this connection: {}", message.into()).into_bytes(),
        }
    }

    fn redirect(location: String) -> Self {
        Self {
            status: 303,
            headers: vec![
                ("Location", location),
                ("Cache-Control", "no-store".into()),
                ("Referrer-Policy", "no-referrer".into()),
            ],
            body: Vec::new(),
        }
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
pub struct AuthorizationServer<'a, S: ?Sized, D: ?Sized> {
    /// Where authorizations are kept.
    pub store: &'a S,
    /// How client metadata documents are fetched.
    pub documents: &'a D,
    /// What registered client IDs are signed under.
    pub key: &'a ClientKey,
    /// This origin, as clients reach it: `https://tonk.foundation`.
    pub origin: &'a str,
    /// The clock, in unix seconds.
    pub now: u64,
}

impl<S, D> AuthorizationServer<'_, S, D>
where
    S: AuthorizationStore + ?Sized,
    D: Documents + ?Sized,
{
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
            ("GET", Endpoint::Authorize) => self.authorize(request).await,
            ("GET", Endpoint::Request(handle)) => self.describe(handle).await,
            ("POST", Endpoint::Request(handle)) => self.answer(handle, request).await,
            ("POST", Endpoint::Token) => self.token(request).await,
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

    /// Validate an authorization request and hand it to the page.
    ///
    /// Until the client and its callback check out, a failure is shown
    /// here: redirecting to an unverified callback would hand an error
    /// (and the user) to whoever wrote it. After that, failures go back
    /// to the client the usual way.
    async fn authorize(&self, request: &Request<'_>) -> Answer {
        let query = request.query.unwrap_or_default();
        if query.len() > MAX_QUERY {
            return Answer::page(400, "the request is too long");
        }
        let Ok(params) = Params::parse(query) else {
            return Answer::page(400, "a parameter is given more than once");
        };
        let Some(client_id) = params.get("client_id") else {
            return Answer::page(400, "no client_id");
        };
        let client = match client::resolve(client_id, self.key, self.documents).await {
            Ok(client) => client,
            Err(ClientError::Unknown(reason) | ClientError::InvalidMetadata(reason)) => {
                return Answer::page(400, reason);
            }
        };
        let Some(redirect_uri) = params.get("redirect_uri") else {
            return Answer::page(400, "no redirect_uri");
        };
        if !client.accepts(redirect_uri) {
            return Answer::page(400, "redirect_uri is not one this client registered");
        }

        let state = params.get("state");
        let refuse = |error: &str, description: &str| {
            Answer::redirect(callback(
                redirect_uri,
                self.origin,
                state,
                &[("error", error), ("error_description", description)],
            ))
        };
        if params.get("response_type") != Some("code") {
            return refuse("unsupported_response_type", "response_type must be code");
        }
        if params.get("code_challenge_method") != Some("S256") {
            return refuse("invalid_request", "PKCE with S256 is required");
        }
        let Some(challenge) = params.get("code_challenge").filter(|c| is_challenge(c)) else {
            return refuse(
                "invalid_request",
                "code_challenge must be an S256 challenge",
            );
        };
        let resource = self.resource();
        if params.get("resource") != Some(resource.as_str()) {
            return refuse("invalid_target", "resource must be this server's /mcp");
        }
        // OAuth 2.1 leaves `state` optional when PKCE carries the CSRF
        // protection, and SDK clients omit it; an absent one is kept as
        // empty and left off the callback.
        let state = state.unwrap_or_default();
        if state.len() > MAX_STATE {
            return refuse("invalid_request", "state is too long");
        }

        let handle = random();
        let browser = random();
        let row = PendingAuthorization {
            request: digest(handle.as_bytes()),
            browser: digest(browser.as_bytes()),
            client_id: client.id,
            client_name: client.name,
            registration: client.registration.as_str().into(),
            redirect_uri: redirect_uri.into(),
            state: state.into(),
            resource,
            challenge: challenge.into(),
            expires_at: self.now + REQUEST_TTL,
        };
        if let Err(error) = self.store.begin_authorization(&row, self.now).await {
            return Answer::page(
                503,
                format!("authorization storage is unavailable: {error}"),
            );
        }
        Answer::redirect(format!("{}{APPROVAL_PAGE}?request={handle}", self.origin)).with(
            "Set-Cookie",
            format!(
                "{}={browser}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{}",
                self.cookie_name(&handle),
                // Outlives the request, so an answer after it lapses is told
                // the request expired rather than that another browser began it.
                REQUEST_TTL + COOKIE_GRACE,
                if self.secure() { "; Secure" } else { "" }
            ),
        )
    }

    /// What the approval page shows: who is asking, and where the
    /// answer goes. Nothing secret, so no cookie is asked for.
    async fn describe(&self, handle: &str) -> Answer {
        match self.pending(handle).await {
            Ok(Some(row)) => Answer::json(
                200,
                json!({
                    "client_id": row.client_id,
                    "client_name": row.client_name,
                    "registration": row.registration,
                    "redirect_uri": row.redirect_uri,
                    "redirect_host": Url::parse(&row.redirect_uri)
                        .ok()
                        .and_then(|url| url.host_str().map(str::to_owned)),
                    "resource": row.resource,
                    "expires_at": row.expires_at,
                }),
            ),
            Ok(None) => gone(),
            Err(answer) => answer,
        }
    }

    /// The approval page's answer: `{"token": "..."}` binds the token to
    /// a fresh code, `{"deny": true}` refuses. Either way the answer is
    /// `{"redirect": "..."}`, the callback to send the browser to.
    ///
    /// Only from this origin, and only from the browser that began the
    /// request: otherwise anyone holding a handle could bind their own
    /// token to someone else's authorization.
    async fn answer(&self, handle: &str, request: &Request<'_>) -> Answer {
        if request.origin != Some(self.origin) {
            return Answer::error(403, "invalid_request", "only the approval page may answer");
        }
        if !media_type_is(request.content_type, "application/json") {
            return Answer::error(400, "invalid_request", "the answer is a JSON body");
        }
        let name = self.cookie_name(handle);
        let Some(browser) = cookie(request.cookie, &name) else {
            return Answer::error(
                403,
                "invalid_request",
                "this browser did not start the authorization",
            );
        };
        let browser = digest(browser.as_bytes());
        let row = match self.pending(handle).await {
            Ok(Some(row)) => row,
            Ok(None) => return gone(),
            Err(answer) => return answer,
        };
        if row.browser != browser {
            return Answer::error(
                403,
                "invalid_request",
                "this browser did not start the authorization",
            );
        }
        let clear = format!(
            "{name}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{}",
            if self.secure() { "; Secure" } else { "" }
        );

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Decision {
            token: Option<String>,
            deny: Option<bool>,
        }
        let decision: Decision = match serde_json::from_slice(request.body) {
            Ok(decision) => decision,
            Err(error) => return Answer::error(400, "invalid_request", error.to_string()),
        };
        let unavailable = |error: crate::store::StoreError| {
            Answer::error(503, "temporarily_unavailable", error.to_string())
        };
        let redirect = match decision {
            Decision {
                token: Some(token),
                deny: None | Some(false),
            } => {
                if !is_bearer_token(&token) {
                    return Answer::error(
                        400,
                        "invalid_request",
                        "the token must be a bearer token (RFC 6750 b64token)",
                    );
                }
                let code = random_bytes();
                let sealed = seal(&code, token.as_bytes());
                match self
                    .store
                    .approve_authorization(
                        &row.request,
                        &browser,
                        &digest(&code),
                        &sealed,
                        self.now,
                        self.now + CODE_TTL,
                    )
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => return gone(),
                    Err(error) => return unavailable(error),
                }
                callback(
                    &row.redirect_uri,
                    self.origin,
                    Some(row.state.as_str()).filter(|state| !state.is_empty()),
                    &[("code", URL_SAFE_NO_PAD.encode(code).as_str())],
                )
            }
            Decision {
                token: None,
                deny: Some(true),
            } => {
                match self
                    .store
                    .deny_authorization(&row.request, &browser, self.now)
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => return gone(),
                    Err(error) => return unavailable(error),
                }
                callback(
                    &row.redirect_uri,
                    self.origin,
                    Some(row.state.as_str()).filter(|state| !state.is_empty()),
                    &[
                        ("error", "access_denied"),
                        ("error_description", "the user refused"),
                    ],
                )
            }
            _ => {
                return Answer::error(
                    400,
                    "invalid_request",
                    "answer with a token or with deny: true",
                );
            }
        };
        Answer::json(200, json!({ "redirect": redirect })).with("Set-Cookie", clear)
    }

    /// The token endpoint (RFC 6749 §4.1.3): the code, its verifier, and
    /// everything it was bound to, for the token. Once.
    async fn token(&self, request: &Request<'_>) -> Answer {
        if !media_type_is(request.content_type, "application/x-www-form-urlencoded") {
            return Answer::error(400, "invalid_request", "the token request is a form body");
        }
        let Ok(body) = std::str::from_utf8(request.body) else {
            return Answer::error(400, "invalid_request", "the body is not UTF-8");
        };
        let Ok(params) = Params::parse(body) else {
            return Answer::error(
                400,
                "invalid_request",
                "a parameter is given more than once",
            );
        };
        if params.get("grant_type") != Some("authorization_code") {
            return Answer::error(
                400,
                "unsupported_grant_type",
                "only authorization_code is supported",
            );
        }
        let (Some(code), Some(client_id), Some(redirect_uri), Some(verifier)) = (
            params.get("code"),
            params.get("client_id"),
            params.get("redirect_uri"),
            params.get("code_verifier"),
        ) else {
            return Answer::error(
                400,
                "invalid_request",
                "code, client_id, redirect_uri and code_verifier are required",
            );
        };
        let resource = self.resource();
        match params.get("resource") {
            None => return Answer::error(400, "invalid_request", "resource is required"),
            Some(asked) if asked != resource => {
                return Answer::error(400, "invalid_target", "resource must be this server's /mcp");
            }
            Some(_) => {}
        }
        let invalid = || {
            Answer::error(
                400,
                "invalid_grant",
                "the code is not valid for this request",
            )
        };
        if !is_verifier(verifier) {
            return invalid();
        }
        let Some(code) = URL_SAFE_NO_PAD
            .decode(code)
            .ok()
            .and_then(|code| <[u8; 32]>::try_from(code).ok())
        else {
            return invalid();
        };
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let sealed = match self
            .store
            .redeem_authorization(Redemption {
                code: &digest(&code),
                client_id,
                redirect_uri,
                resource: &resource,
                challenge: &challenge,
                now: self.now,
            })
            .await
        {
            Ok(Some(sealed)) => sealed,
            Ok(None) => return invalid(),
            Err(error) => return Answer::error(503, "temporarily_unavailable", error.to_string()),
        };
        let Some(token) = open(&code, &sealed) else {
            return Answer::error(500, "server_error", "the stored token could not be opened");
        };
        // No `expires_in`, no `refresh_token`: see the module docs.
        Answer::json(
            200,
            json!({ "access_token": token, "token_type": "Bearer" }),
        )
        .with("Pragma", "no-cache".into())
    }

    async fn pending(&self, handle: &str) -> Result<Option<PendingAuthorization>, Answer> {
        self.store
            .pending_authorization(&digest(handle.as_bytes()), self.now)
            .await
            .map_err(|error| Answer::error(503, "temporarily_unavailable", error.to_string()))
    }

    /// Whether this origin is served over TLS. A plain `http://localhost`
    /// origin in development cannot keep a `Secure` cookie name.
    fn secure(&self) -> bool {
        self.origin.starts_with("https://")
    }

    /// Each request has a cookie of its own, so a second authorization in
    /// another tab does not unbind the first.
    fn cookie_name(&self, handle: &str) -> String {
        let prefix = if self.secure() { "__Host-" } else { "" };
        format!("{prefix}tonk-authorize-{handle}")
    }
}

/// A request that is not pending: never was, lapsed, or already
/// answered.
fn gone() -> Answer {
    Answer::error(
        404,
        "invalid_request",
        "this authorization is not pending; start again from the client",
    )
}

/// Query or form parameters, refusing repeats (RFC 6749 §3.1).
struct Params(Vec<(String, String)>);

impl Params {
    fn parse(input: &str) -> Result<Self, ()> {
        let mut params: Vec<(String, String)> = Vec::new();
        for (key, value) in url::form_urlencoded::parse(input.as_bytes()) {
            if params.iter().any(|(seen, _)| *seen == key) {
                return Err(());
            }
            params.push((key.into_owned(), value.into_owned()));
        }
        Ok(Self(params))
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// `uri` with `params`, `state` and `iss` (RFC 9207) added to its query.
fn callback(uri: &str, issuer: &str, state: Option<&str>, params: &[(&str, &str)]) -> String {
    let mut url = Url::parse(uri).expect("a validated redirect_uri parses");
    {
        let mut query = url.query_pairs_mut();
        for (name, value) in params {
            query.append_pair(name, value);
        }
        if let Some(state) = state {
            query.append_pair("state", state);
        }
        query.append_pair("iss", issuer);
    }
    url.into()
}

/// Whether `content_type` names `expected`, parameters aside.
fn media_type_is(content_type: Option<&str>, expected: &str) -> bool {
    content_type
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case(expected))
}

/// The value of the cookie `name`, when exactly one is sent.
fn cookie<'a>(header: Option<&'a str>, name: &str) -> Option<&'a str> {
    let mut found = header?
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .filter(|(key, _)| *key == name)
        .map(|(_, value)| value);
    let value = found.next()?;
    found.next().is_none().then_some(value)
}

/// An S256 challenge: the unpadded base64url of a SHA-256.
fn is_challenge(value: &str) -> bool {
    value.len() == 43
        && URL_SAFE_NO_PAD
            .decode(value)
            .is_ok_and(|bytes| bytes.len() == 32)
}

/// A PKCE verifier (RFC 7636 §4.1): 43 to 128 unreserved characters.
fn is_verifier(value: &str) -> bool {
    (43..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
}

/// RFC 6750 `b64token`: what an `Authorization: Bearer` header can
/// carry. The only thing this server asks of a token.
fn is_bearer_token(value: &str) -> bool {
    let body = value.trim_end_matches('=');
    !body.is_empty()
        && value.len() <= MAX_TOKEN
        && body
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~+/".contains(&b))
}

fn random_bytes() -> [u8; 32] {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the platform has an entropy source");
    bytes
}

/// 32 random bytes, base64url: a handle, a cookie, a code.
fn random() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes())
}

/// Hex SHA-256, the form every secret is stored in.
fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// The cipher for one code's token. The code is 32 random bytes used
/// once, so its key seals exactly one message and a fixed nonce is safe.
fn cipher(code: &[u8; 32]) -> Aes256Gcm {
    let mut key = [0u8; 32];
    Hkdf::<Sha256>::new(None, code)
        .expand(SEAL_CONTEXT, &mut key)
        .expect("32 bytes is a valid HKDF output");
    Aes256Gcm::new_from_slice(&key).expect("AES-256 takes a 32-byte key")
}

/// Seal `token` under `code`, as base64url.
fn seal(code: &[u8; 32], token: &[u8]) -> String {
    let sealed = cipher(code)
        .encrypt(&Nonce::from([0u8; 12]), token)
        .expect("AES-GCM seals any message this size");
    URL_SAFE_NO_PAD.encode(sealed)
}

/// Open what [`seal`] sealed.
fn open(code: &[u8; 32], sealed: &str) -> Option<String> {
    let sealed = URL_SAFE_NO_PAD.decode(sealed).ok()?;
    let token = cipher(code)
        .decrypt(&Nonce::from([0u8; 12]), sealed.as_slice())
        .ok()?;
    String::from_utf8(token).ok()
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests;
