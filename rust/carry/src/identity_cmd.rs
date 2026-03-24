//! `carry identity` — manage the local user identity.
//!
//! Identity is derived from a WebAuthn passkey via the PRF extension. The CLI
//! starts a local HTTP server, opens a browser for the passkey ceremony, and
//! receives the PRF output. The output is fed through HKDF-SHA256 to produce
//! a deterministic Ed25519 keypair, cached at `~/.carry/identity`.

use anyhow::{Context, Result};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use axum::routing::{get, post};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tonk_space::Operator;

/// The HTML page for the WebAuthn ceremony.
const IDENTITY_PAGE: &str = include_str!("identity_page.html");

/// Filename for the cached identity key.
const IDENTITY_FILE: &str = "identity";

/// Filename for the stored WebAuthn credential ID.
const CREDENTIAL_FILE: &str = "credential";

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Get the carry home directory (`~/.carry/`), creating it if needed.
fn carry_home() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    let carry_dir = home.join(".carry");
    if !carry_dir.exists() {
        std::fs::create_dir_all(&carry_dir)
            .with_context(|| format!("Failed to create {}", carry_dir.display()))?;
    }
    Ok(carry_dir)
}

/// Load a cached identity from `~/.carry/identity`, if it exists.
pub fn load_identity() -> Result<Option<Operator>> {
    let path = carry_home()?.join(IDENTITY_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let bytes =
        std::fs::read(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    if bytes.len() != 32 {
        anyhow::bail!(
            "Invalid identity file at {} (expected 32 bytes, got {})",
            path.display(),
            bytes.len()
        );
    }
    let mut secret = [0u8; 32];
    secret.copy_from_slice(&bytes);
    Ok(Some(Operator::from_secret(secret)))
}

/// Load identity or error with a helpful message.
pub fn require_identity() -> Result<Operator> {
    load_identity()?.context("No local identity found. Run `carry identity` to create one.")
}

/// Save an identity to `~/.carry/identity`.
fn save_identity(operator: &Operator) -> Result<()> {
    let path = carry_home()?.join(IDENTITY_FILE);
    std::fs::write(&path, operator.to_secret())
        .with_context(|| format!("Failed to write {}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

/// Load a stored WebAuthn credential ID, if it exists.
fn load_credential_id() -> Result<Option<String>> {
    let path = carry_home()?.join(CREDENTIAL_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let trimmed = content.trim().to_string();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed))
    }
}

/// Save a WebAuthn credential ID.
fn save_credential_id(id: &str) -> Result<()> {
    let path = carry_home()?.join(CREDENTIAL_FILE);
    std::fs::write(&path, id).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// CLI execute
// ---------------------------------------------------------------------------

/// Execute `carry identity [--reset]`.
///
/// If `reset` is false and a cached identity exists, just print the DID.
/// Otherwise, run the passkey browser flow.
pub async fn execute(reset: bool) -> Result<()> {
    if !reset && let Some(operator) = load_identity()? {
        println!("{}", operator.did());
        return Ok(());
    }

    // Run the passkey browser flow
    let prf_output = run_passkey_flow().await?;

    // Derive identity from PRF output via HKDF
    let operator = Operator::from_passphrase(&prf_output).await;

    // Cache the derived identity
    save_identity(&operator)?;

    println!("{}", operator.did());

    Ok(())
}

// ---------------------------------------------------------------------------
// Passkey browser flow
// ---------------------------------------------------------------------------

/// Info sent to the browser page to decide register vs authenticate.
#[derive(Serialize)]
struct SessionInfo {
    credential_id: Option<String>,
    rp_id: String,
    user_id: String,
    challenge: String,
}

/// Callback payload from the browser page.
#[derive(Deserialize)]
struct CallbackPayload {
    credential_id: Option<String>,
    prf_output: Option<String>,
    phase: Option<String>,
    error: Option<String>,
}

/// Shared state for the axum server.
struct ServerState {
    credential_id: Option<String>,
    challenge: String,
    tx: std::sync::Mutex<Option<oneshot::Sender<Result<CallbackPayload>>>>,
}

/// Start a local HTTP server, open the browser, wait for the PRF result.
async fn run_passkey_flow() -> Result<String> {
    let credential_id = load_credential_id()?;

    // Generate a random challenge for registration
    let mut challenge_bytes = [0u8; 32];
    use rand_0_8::RngCore;
    rand_0_8::rngs::OsRng.fill_bytes(&mut challenge_bytes);
    let challenge = URL_SAFE_NO_PAD.encode(challenge_bytes);

    // Random user ID for registration (only used once, not security-critical)
    let mut user_id_bytes = [0u8; 16];
    rand_0_8::rngs::OsRng.fill_bytes(&mut user_id_bytes);
    let user_id = URL_SAFE_NO_PAD.encode(user_id_bytes);

    let (tx, rx) = oneshot::channel();

    let state = Arc::new(ServerState {
        credential_id: credential_id.clone(),
        challenge: challenge.clone(),
        tx: std::sync::Mutex::new(Some(tx)),
    });

    let app = axum::Router::new()
        .route("/", get(serve_page))
        .route("/auth", get(serve_page))
        .route(
            "/info",
            get({
                let user_id = user_id.clone();
                move |State(state): State<Arc<ServerState>>| async move {
                    Json(SessionInfo {
                        credential_id: state.credential_id.clone(),
                        rp_id: "localhost".to_string(),
                        user_id,
                        challenge: state.challenge.clone(),
                    })
                }
            }),
        )
        .route("/callback", post(handle_callback))
        .with_state(state);

    // Bind to ephemeral port
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let url = format!("http://localhost:{}/auth", addr.port());

    eprintln!("Opening browser for passkey authentication...");

    // Open browser
    if webbrowser::open(&url).is_err() {
        eprintln!("Could not open browser automatically.");
        eprintln!("Please open this URL manually: {}", url);
    }

    // Serve until we get the callback
    let server = axum::serve(listener, app).into_future();
    tokio::select! {
        result = rx => {
            let payload: CallbackPayload = result.context("Server channel closed unexpectedly")??;
            process_callback(payload).await
        }
        result = server => {
            let _: Result<(), std::io::Error> = result;
            anyhow::bail!("Server exited without receiving callback")
        }
    }
}

async fn serve_page() -> Html<&'static str> {
    Html(IDENTITY_PAGE)
}

async fn handle_callback(
    State(state): State<Arc<ServerState>>,
    Json(payload): Json<CallbackPayload>,
) -> StatusCode {
    if let Some(tx) = state.tx.lock().unwrap().take() {
        let _ = tx.send(Ok(payload));
        StatusCode::OK
    } else {
        StatusCode::GONE
    }
}

/// Process the callback payload: save credential ID, return PRF output as hex.
async fn process_callback(payload: CallbackPayload) -> Result<String> {
    // Check for errors from the browser
    if let Some(error) = payload.error {
        anyhow::bail!("Passkey authentication failed: {}", error);
    }

    let prf_b64u = payload.prf_output.context("No PRF output in callback")?;

    let prf_bytes = URL_SAFE_NO_PAD
        .decode(&prf_b64u)
        .context("Invalid base64url in PRF output")?;

    // Save credential ID for future authentications
    if let Some(ref cred_id) = payload.credential_id {
        save_credential_id(cred_id)?;
    }

    let phase = payload.phase.as_deref().unwrap_or("unknown");
    eprintln!("Passkey {} successful.", phase);

    // Return PRF output as hex string (used as passphrase input to HKDF)
    Ok(hex::encode(&prf_bytes))
}
