use crate::delegation::{Delegation, DelegationMetadata, keypair_to_signer};
use crate::keystore::Keystore;
use anyhow::{Context, Result};
use axum::{
    Router,
    extract::{Form, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
};
use base64::Engine as _;
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Notify;
use ucan::Delegation as UcanDelegation;
use ucan::delegation::subject::DelegatedSubject;
use ucan::did::Ed25519Did;

const AUTH_HTML: &str = include_str!("../auth.html");

#[derive(Deserialize)]
struct CallbackForm {
    #[serde(default)]
    authorize: Option<String>,
    #[serde(default)]
    deny: Option<String>,
}

#[derive(Clone)]
struct AppState {
    shutdown: Arc<Notify>,
    operator_did: Arc<String>,
    auth_url: Arc<String>,
}

/// Execute the login flow
pub async fn execute(via: Option<String>) -> Result<()> {
    println!("🔐 Login...\n");

    // Get or create operator keypair
    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;

    // Generate operator DID
    let operator_did = operator.did().to_string();
    println!("🫆 Operator: {}\n", operator_did);

    // Find available port for callback server
    let callback_port = find_available_port(8089)?;
    let callback_url = format!("http://localhost:{}", callback_port);

    // Build auth URL and determine auth site URL
    let (auth_url, auth_site_url) = match &via {
        Some(base_url) => {
            // Use provided auth URL
            let url = format!(
                "{}?as={}&cmd=/&sub=*&callback={}",
                base_url, operator_did, callback_url
            );
            (url, base_url.clone())
        }
        None => {
            // Start local auth server
            let auth_port = find_available_port(8088)?;
            let auth_addr = SocketAddr::from(([127, 0, 0, 1], auth_port));

            println!("🌐 Starting local auth server on {}...", auth_addr);
            tokio::spawn(async move {
                let app = Router::new().route("/", get(serve_auth_html));
                let listener = tokio::net::TcpListener::bind(auth_addr)
                    .await
                    .expect("Failed to bind auth server");
                axum::serve(listener, app)
                    .await
                    .expect("Auth server failed");
            });

            let url = format!(
                "http://localhost:{}?as={}&cmd=/&sub=*&callback={}",
                auth_port, operator_did, callback_url
            );
            let site = format!("http://localhost:{}", auth_port);
            (url, site)
        }
    };

    // Start callback server
    let shutdown_signal = Arc::new(Notify::new());
    let state = AppState {
        shutdown: shutdown_signal.clone(),
        operator_did: Arc::new(operator_did.clone()),
        auth_url: Arc::new(auth_site_url.clone()),
    };

    let callback_addr = SocketAddr::from(([127, 0, 0, 1], callback_port));
    println!("📞 Starting callback server on {}...\n", callback_addr);

    let app = Router::new()
        .route("/", post(handle_callback))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(callback_addr)
        .await
        .context("Failed to bind callback server")?;

    // Open browser
    println!("🌐 Opening browser for authentication...");
    println!("   URL: {}\n", auth_url);

    if let Err(e) = webbrowser::open(&auth_url) {
        println!("⚠ Failed to open browser automatically: {}", e);
        println!("   Please open this URL manually: {}", auth_url);
    }

    // Serve callback endpoint with graceful shutdown
    println!("⏳ Waiting for authorization (timeout: 5 minutes)...\n");

    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        shutdown_signal.notified().await;
    });

    // Run server with timeout
    let result = tokio::time::timeout(std::time::Duration::from_secs(300), server).await;

    match result {
        Ok(Ok(_)) => {
            println!("\n✅ Login complete!");
        }
        Ok(Err(e)) => {
            anyhow::bail!("Server error: {}", e);
        }
        Err(_) => {
            anyhow::bail!("Timeout waiting for authorization");
        }
    }

    Ok(())
}

/// Execute a self-issued login (agent/non-interactive mode).
/// The operator creates a powerline delegation to itself, becoming its own authority.
pub async fn execute_self() -> Result<()> {
    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;
    let operator_did = operator.did().to_string();

    println!("🤖 Self-auth mode: operator becomes its own authority\n");
    println!("🫆 Operator: {}\n", operator_did);

    // Create a powerline delegation: operator → operator (self-issued, no expiry)
    let signer = keypair_to_signer(&operator);
    let audience_did: Ed25519Did = operator_did
        .parse()
        .map_err(|e| anyhow::anyhow!("Failed to parse operator DID: {:?}", e))?;

    let ucan_delegation: UcanDelegation<Ed25519Did> = UcanDelegation::builder()
        .issuer(signer)
        .audience(audience_did)
        .subject(DelegatedSubject::Any) // Powerline
        .command(vec!["/".to_string()])
        .try_build()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to build self-delegation: {}", e))?;

    let delegation = Delegation::from_ucan(ucan_delegation);

    // Save delegation with metadata
    let metadata = DelegationMetadata {
        site: "self".to_string(),
        received_at: chrono::Utc::now(),
        is_local: true,
        extra: serde_json::Value::Null,
    };

    delegation
        .save_with_metadata(&metadata)
        .context("Failed to save self-delegation")?;

    // Create and save session metadata
    let session_meta =
        crate::metadata::SessionMetadata::new(operator_did.clone(), "self".to_string());
    session_meta
        .save(&operator_did)
        .context("Failed to save session metadata")?;

    // Set as active session
    crate::state::set_active_session(&operator_did)
        .context("Failed to set active session")?;

    println!("✅ Self-auth complete!");
    println!("   Authority: {}", operator_did);
    println!("   Session is active. You can now create spaces and work with facts.\n");

    Ok(())
}

/// Import a delegation from a file path or base64-encoded string.
pub async fn execute_import(input: &str) -> Result<()> {
    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;
    let operator_did = operator.did().to_string();

    println!("📥 Importing delegation...\n");
    println!("🫆 Operator: {}\n", operator_did);

    // Read delegation from file or decode from base64
    let decoded = if std::path::Path::new(input).exists() {
        println!("📂 Reading from file: {}", input);
        std::fs::read(input).context("Failed to read delegation file")?
    } else {
        println!("📥 Decoding base64...");
        base64::engine::general_purpose::STANDARD
            .decode(input)
            .context("Failed to decode base64. Input is neither a valid file path nor base64.")?
    };

    // Parse the delegation
    let delegation =
        Delegation::from_cbor_bytes(&decoded).context("Failed to parse delegation CBOR")?;

    // Validate audience matches operator DID
    if delegation.audience() != operator_did {
        anyhow::bail!(
            "Delegation audience mismatch!\n   Expected (this operator): {}\n   Got: {}\n   The delegation must be issued TO this operator's DID.",
            operator_did,
            delegation.audience()
        );
    }

    // Validate not expired
    if !delegation.is_valid() {
        anyhow::bail!("Delegation is expired or not yet valid.");
    }

    println!("✅ Delegation valid!");
    println!("   Issuer:   {}", delegation.issuer());
    println!("   Audience: {}", delegation.audience());
    println!("   Command:  {}", delegation.command_str());
    println!(
        "   Subject:  {}",
        match delegation.subject() {
            DelegatedSubject::Specific(did) => did.to_string(),
            DelegatedSubject::Any => "*".to_string(),
        }
    );

    // Save delegation with metadata
    let metadata = DelegationMetadata {
        site: "import".to_string(),
        received_at: chrono::Utc::now(),
        is_local: true,
        extra: serde_json::Value::Null,
    };

    delegation
        .save_raw_with_metadata(&decoded, &metadata)
        .context("Failed to save delegation")?;

    // Create and save session metadata using the issuer as authority
    let authority_did = delegation.issuer();
    let session_meta =
        crate::metadata::SessionMetadata::new(authority_did.clone(), "import".to_string());
    session_meta
        .save(&authority_did)
        .context("Failed to save session metadata")?;

    // Set as active session
    crate::state::set_active_session(&authority_did)
        .context("Failed to set active session")?;

    println!("\n✅ Delegation imported and session activated!");
    println!("   Authority: {}\n", authority_did);

    Ok(())
}

async fn serve_auth_html() -> Html<&'static str> {
    Html(AUTH_HTML)
}

async fn handle_callback(
    State(state): State<AppState>,
    Form(form): Form<CallbackForm>,
) -> impl IntoResponse {
    if let Some(deny) = form.deny {
        println!("❌ Authorization denied: {}", deny);
        state.shutdown.notify_one();
        return (
            StatusCode::OK,
            Html(
                "<html><body><h1>Authorization Denied</h1><p>You can close this window.</p></body></html>",
            ),
        );
    }

    if let Some(authorize) = form.authorize {
        // Decode base64-encoded UCAN
        let decoded = match base64::engine::general_purpose::STANDARD.decode(&authorize) {
            Ok(bytes) => bytes,
            Err(e) => {
                println!("❌ Failed to decode base64: {}", e);
                state.shutdown.notify_one();
                return (
                    StatusCode::BAD_REQUEST,
                    Html("<html><body><h1>Error</h1><p>Invalid base64 encoding.</p></body></html>"),
                );
            }
        };

        // Parse DAG-CBOR encoded UCAN
        match Delegation::from_cbor_bytes(&decoded) {
            Ok(delegation) => {
                println!("✅ Received delegation!");
                println!("   Issuer: {}", delegation.issuer());
                println!("   Audience: {}", delegation.audience());
                println!("   Command: {}", delegation.command_str());

                // Validate audience matches operator DID
                if delegation.audience() != state.operator_did.as_str() {
                    println!("❌ Delegation audience mismatch!");
                    println!("   Expected: {}", state.operator_did);
                    println!("   Got: {}", delegation.audience());
                    state.shutdown.notify_one();
                    return (
                        StatusCode::BAD_REQUEST,
                        Html(
                            "<html><body><h1>Error</h1><p>Delegation audience mismatch.</p></body></html>",
                        ),
                    );
                }

                // Validate not expired
                if !delegation.is_valid() {
                    println!("❌ Delegation is already expired!");
                    state.shutdown.notify_one();
                    return (
                        StatusCode::BAD_REQUEST,
                        Html(
                            "<html><body><h1>Error</h1><p>Delegation is expired.</p></body></html>",
                        ),
                    );
                }

                // Create metadata
                let metadata = DelegationMetadata {
                    site: state.auth_url.to_string(),
                    received_at: chrono::Utc::now(),
                    is_local: state.auth_url.starts_with("http://localhost")
                        || state.auth_url.starts_with("http://127.0.0.1"),
                    extra: serde_json::Value::Null,
                };

                // Save delegation with metadata using raw bytes to preserve exact format
                if let Err(e) = delegation.save_raw_with_metadata(&decoded, &metadata) {
                    println!("⚠ Failed to save delegation: {}", e);
                    state.shutdown.notify_one();
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Html(
                            "<html><body><h1>Error</h1><p>Failed to save delegation.</p></body></html>",
                        ),
                    );
                }

                // Create and save session metadata
                let authority_did = delegation.issuer();
                let session_meta = crate::metadata::SessionMetadata::new(
                    authority_did.clone(),
                    state.auth_url.to_string(),
                );
                if let Err(e) = session_meta.save(&authority_did) {
                    println!("⚠ Failed to save session metadata: {}", e);
                }

                // Set as active session
                if let Err(e) = crate::state::set_active_session(&authority_did) {
                    println!("⚠ Failed to set active session: {}", e);
                }

                // Trigger shutdown
                state.shutdown.notify_one();

                (
                    StatusCode::OK,
                    Html(
                        "<html><body><h1>✅ Authorization Successful!</h1><p>You can close this window and return to the CLI.</p></body></html>",
                    ),
                )
            }
            Err(e) => {
                println!("❌ Failed to parse delegation: {}", e);
                state.shutdown.notify_one();
                (
                    StatusCode::BAD_REQUEST,
                    Html(
                        "<html><body><h1>Error</h1><p>Failed to parse delegation.</p></body></html>",
                    ),
                )
            }
        }
    } else {
        state.shutdown.notify_one();
        (
            StatusCode::BAD_REQUEST,
            Html(
                "<html><body><h1>Error</h1><p>No authorization or denial received.</p></body></html>",
            ),
        )
    }
}

fn find_available_port(preferred: u16) -> Result<u16> {
    // Try preferred port first
    if port_is_available(preferred) {
        return Ok(preferred);
    }

    // Find any available port
    for port in 8000..9000 {
        if port_is_available(port) {
            return Ok(port);
        }
    }

    anyhow::bail!("No available ports found")
}

fn port_is_available(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}
