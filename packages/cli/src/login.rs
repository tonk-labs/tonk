use crate::delegation::{Delegation, DelegationMetadata};
use crate::keystore::Keystore;
use anyhow::{Context, Result};
use axum::{
    extract::{Form, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Router,
};
use base64::Engine as _;
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Notify;

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

/// Parse duration string (e.g., "30d", "7d", "1h") into seconds
fn parse_duration(duration_str: &str) -> Result<i64> {
    let duration_str = duration_str.trim();
    if duration_str.is_empty() {
        anyhow::bail!("Duration string is empty");
    }

    let (num_str, unit) = duration_str.split_at(duration_str.len() - 1);
    let num: i64 = num_str.parse().context("Invalid duration number")?;

    let seconds = match unit {
        "s" => num,
        "m" => num * 60,
        "h" => num * 3600,
        "d" => num * 86400,
        _ => anyhow::bail!("Invalid duration unit. Use 's', 'm', 'h', or 'd'"),
    };

    Ok(seconds)
}

/// Execute the login flow
pub async fn execute(via: Option<String>, duration: String) -> Result<()> {
    println!("🔐 Login...\n");

    // Parse duration
    let duration_secs = parse_duration(&duration).context("Failed to parse duration")?;
    println!(
        "📅 Session duration: {} ({} seconds)",
        duration, duration_secs
    );

    // Get or create operator keypair
    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;

    // Generate operator DID
    let operator_did = operator.to_did_key();
    println!("🤖 Operator: {}\n", operator_did);

    // Find available port for callback server
    let callback_port = find_available_port(8089)?;
    let callback_url = format!("http://localhost:{}", callback_port);

    // Build auth URL and determine auth site URL
    let (auth_url, auth_site_url) = match &via {
        Some(base_url) => {
            // Use provided auth URL
            let url = format!(
                "{}?as={}&cmd=/&sub=null&callback={}&duration={}",
                base_url, operator_did, callback_url, duration_secs
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
                "http://localhost:{}?as={}&cmd=/&sub=null&callback={}&duration={}",
                auth_port, operator_did, callback_url, duration_secs
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
            Html("<html><body><h1>Authorization Denied</h1><p>You can close this window.</p></body></html>"),
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
                        Html("<html><body><h1>Error</h1><p>Delegation audience mismatch.</p></body></html>"),
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

                // Save delegation with metadata
                if let Err(e) = delegation.save_with_metadata(&metadata) {
                    println!("⚠ Failed to save delegation: {}", e);
                    state.shutdown.notify_one();
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Html("<html><body><h1>Error</h1><p>Failed to save delegation.</p></body></html>"),
                    );
                }

                // Trigger shutdown
                state.shutdown.notify_one();

                (
                    StatusCode::OK,
                    Html("<html><body><h1>✅ Authorization Successful!</h1><p>You can close this window and return to the CLI.</p></body></html>"),
                )
            }
            Err(e) => {
                println!("❌ Failed to parse delegation: {}", e);
                state.shutdown.notify_one();
                (
                    StatusCode::BAD_REQUEST,
                    Html("<html><body><h1>Error</h1><p>Failed to parse delegation.</p></body></html>"),
                )
            }
        }
    } else {
        state.shutdown.notify_one();
        (
            StatusCode::BAD_REQUEST,
            Html("<html><body><h1>Error</h1><p>No authorization or denial received.</p></body></html>"),
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
