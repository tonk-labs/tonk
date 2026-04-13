//! Passkey-based identity derivation via WebAuthn PRF.
//!
//! Spins up a localhost HTTP server, opens the user's browser to perform a
//! WebAuthn ceremony with the PRF extension, and derives deterministic
//! Ed25519 keys from the PRF output.

use anyhow::{Context, Result, bail};
use dialog_credentials::{Ed25519Signer, SignerCredential};
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

const IDENTITY_PAGE: &str = include_str!("identity_page.html");

/// Result of a successful passkey ceremony.
pub struct PasskeyResult {
    /// The raw 32-byte PRF output.
    pub prf_output: [u8; 32],
    /// The WebAuthn credential ID (base64url-encoded) for future authentication.
    pub credential_id: String,
}

/// Keys derived from a passkey PRF output.
pub struct DerivedIdentity {
    pub account_signer: SignerCredential,
    pub profile_signer: SignerCredential,
}

/// Derive account and profile signers from a PRF output.
///
/// Uses blake3 key derivation with distinct context strings so the same
/// PRF output yields two independent Ed25519 keys.
pub async fn derive_identity(prf_output: &[u8; 32]) -> Result<DerivedIdentity> {
    let account_seed = blake3::derive_key("carry account v1", prf_output);
    let profile_seed = blake3::derive_key("carry profile v1", prf_output);

    let account_signer = Ed25519Signer::import(&account_seed)
        .await
        .context("Failed to derive account key from PRF output")?;
    let profile_signer = Ed25519Signer::import(&profile_seed)
        .await
        .context("Failed to derive profile key from PRF output")?;

    Ok(DerivedIdentity {
        account_signer: SignerCredential::from(account_signer),
        profile_signer: SignerCredential::from(profile_signer),
    })
}

/// Run the passkey WebAuthn ceremony via a local browser.
///
/// If `credential_id` is `Some`, uses the authentication flow (returning user).
/// Otherwise uses the registration flow (new passkey).
pub async fn authenticate(credential_id: Option<&str>) -> Result<PasskeyResult> {
    let (tx, rx) = oneshot::channel::<Result<CallbackPayload>>();
    let tx = Arc::new(tokio::sync::Mutex::new(Some(tx)));

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("Failed to bind local server")?;
    let addr = listener.local_addr()?;

    let session = Arc::new(Session {
        credential_id: credential_id.map(String::from),
    });

    let server_handle = tokio::spawn({
        let tx = tx.clone();
        let session = session.clone();
        async move {
            // Accept connections until the callback is received.
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(conn) => conn,
                    Err(_) => break,
                };
                let tx = tx.clone();
                let session = session.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |req| {
                        let tx = tx.clone();
                        let session = session.clone();
                        async move { handle_request(req, &session, tx).await }
                    });
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        }
    });

    let url = format!("http://localhost:{}", addr.port());
    eprintln!("Opening browser for passkey authentication...");
    eprintln!("If the browser doesn't open, visit: {}", url);

    if open::that(&url).is_err() {
        eprintln!("Could not open browser automatically.");
    }

    let payload = tokio::time::timeout(std::time::Duration::from_secs(300), rx)
        .await
        .map_err(|_| anyhow::anyhow!("Passkey ceremony timed out after 5 minutes"))?
        .context("Passkey server shut down unexpectedly")?
        .context("Passkey authentication failed")?;

    server_handle.abort();

    let prf_bytes =
        base64url_decode(&payload.prf_output).context("Invalid base64url in PRF output")?;

    if prf_bytes.len() != 32 {
        bail!("PRF output is {} bytes, expected 32", prf_bytes.len());
    }

    let mut prf_output = [0u8; 32];
    prf_output.copy_from_slice(&prf_bytes);

    Ok(PasskeyResult {
        prf_output,
        credential_id: payload.credential_id,
    })
}

/// Load a stored passkey credential ID from the profile directory.
pub fn load_credential_id(profile_dir: &Path) -> Option<String> {
    let path = profile_dir.join("passkey");
    std::fs::read_to_string(&path).ok()
}

/// Save a passkey credential ID to the profile directory.
pub fn save_credential_id(profile_dir: &Path, credential_id: &str) -> Result<()> {
    std::fs::create_dir_all(profile_dir)
        .with_context(|| format!("Failed to create {}", profile_dir.display()))?;
    std::fs::write(profile_dir.join("passkey"), credential_id)
        .context("Failed to save passkey credential ID")
}

/// Save the account DID string to the profile directory.
pub fn save_account_did(profile_dir: &Path, account_did: &str) -> Result<()> {
    std::fs::create_dir_all(profile_dir)
        .with_context(|| format!("Failed to create {}", profile_dir.display()))?;
    std::fs::write(profile_dir.join("account-did"), account_did)
        .context("Failed to save account DID")
}

/// Load a stored account DID from the profile directory.
pub fn load_account_did(profile_dir: &Path) -> Option<String> {
    let path = profile_dir.join("account-did");
    std::fs::read_to_string(&path).ok()
}

struct Session {
    credential_id: Option<String>,
}

#[derive(Deserialize)]
struct CallbackPayload {
    credential_id: String,
    prf_output: String,
    #[allow(dead_code)]
    phase: String,
}

#[derive(Deserialize)]
struct CallbackError {
    error: String,
}

#[derive(Serialize)]
struct InfoResponseAuth {
    credential_id: String,
}

#[derive(Serialize)]
struct InfoResponseRegister {
    rp_id: String,
    user_id: String,
    challenge: String,
}

type BoxBody = Full<bytes::Bytes>;
type HyperResponse = Response<BoxBody>;

fn json_response(status: StatusCode, body: &impl Serialize) -> Result<HyperResponse, hyper::Error> {
    let json = serde_json::to_string(body).unwrap();
    Ok(Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Full::new(bytes::Bytes::from(json)))
        .unwrap())
}

fn text_response(status: StatusCode, text: &str) -> Result<HyperResponse, hyper::Error> {
    Ok(Response::builder()
        .status(status)
        .header("Content-Type", "text/plain")
        .body(Full::new(bytes::Bytes::from(text.to_string())))
        .unwrap())
}

fn html_response(html: &str) -> Result<HyperResponse, hyper::Error> {
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/html; charset=utf-8")
        .body(Full::new(bytes::Bytes::from(html.to_string())))
        .unwrap())
}

async fn handle_request(
    req: Request<Incoming>,
    session: &Session,
    tx: Arc<tokio::sync::Mutex<Option<oneshot::Sender<Result<CallbackPayload>>>>>,
) -> Result<HyperResponse, hyper::Error> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/") => html_response(IDENTITY_PAGE),

        (&Method::GET, "/info") => {
            if let Some(ref cred_id) = session.credential_id {
                json_response(
                    StatusCode::OK,
                    &InfoResponseAuth {
                        credential_id: cred_id.clone(),
                    },
                )
            } else {
                let mut user_id = [0u8; 32];
                getrandom::fill(&mut user_id).unwrap();
                let mut challenge = [0u8; 32];
                getrandom::fill(&mut challenge).unwrap();

                json_response(
                    StatusCode::OK,
                    &InfoResponseRegister {
                        rp_id: "localhost".to_string(),
                        user_id: base64url_encode(&user_id),
                        challenge: base64url_encode(&challenge),
                    },
                )
            }
        }

        (&Method::POST, "/callback") => {
            use http_body_util::BodyExt;
            let body = req.into_body().collect().await.unwrap().to_bytes();
            let body_str = String::from_utf8_lossy(&body);

            // Check if this is an error response
            if let Ok(err) = serde_json::from_str::<CallbackError>(&body_str) {
                if let Some(tx) = tx.lock().await.take() {
                    let _ = tx.send(Err(anyhow::anyhow!("Passkey error: {}", err.error)));
                }
                return text_response(StatusCode::OK, "error received");
            }

            match serde_json::from_str::<CallbackPayload>(&body_str) {
                Ok(payload) => {
                    if let Some(tx) = tx.lock().await.take() {
                        let _ = tx.send(Ok(payload));
                    }
                    text_response(StatusCode::OK, "ok")
                }
                Err(e) => text_response(
                    StatusCode::BAD_REQUEST,
                    &format!("Invalid callback payload: {}", e),
                ),
            }
        }

        _ => text_response(StatusCode::NOT_FOUND, "not found"),
    }
}

fn base64url_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn base64url_decode(s: &str) -> Result<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s)
        .context("base64url decode failed")
}
