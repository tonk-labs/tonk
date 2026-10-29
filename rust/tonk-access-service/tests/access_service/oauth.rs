//! The OAuth authorization server against the local server, over HTTP:
//! what an MCP client does first (discovery, registration).
//!
//! The protocol's refusals are tested beside it, in
//! `src/oauth/tests.rs`; this checks the wire: headers, and that the
//! native server routes what the worker routes.

#![cfg(all(feature = "helpers", not(target_arch = "wasm32")))]

use serde_json::{Value, json};
use tonk_access_service::helpers::{AccessServiceSettings, access_service};

#[dialog_common::test]
async fn it_serves_discovery_and_registration() -> anyhow::Result<()> {
    let service = access_service(AccessServiceSettings::default()).await?;
    let origin = service
        .address
        .access_service_url
        .trim_end_matches('/')
        .to_owned();
    let client = reqwest::Client::new();

    // Discovery, from the resource to its server.
    let resource: Value = client
        .get(format!("{origin}/.well-known/oauth-protected-resource/mcp"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(resource["resource"], format!("{origin}/mcp"));
    assert_eq!(resource["authorization_servers"], json!([origin]));
    let server: Value = client
        .get(format!("{origin}/.well-known/oauth-authorization-server"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(server["issuer"], origin);

    // Registration.
    let registered = client
        .post(server["registration_endpoint"].as_str().unwrap())
        .json(&json!({
            "client_name": "Test client",
            "redirect_uris": ["http://127.0.0.1/callback"],
        }))
        .send()
        .await?;
    assert_eq!(registered.status(), 201);
    assert_eq!(registered.headers()["cache-control"], "no-store");
    let registered: Value = registered.json().await?;
    assert_eq!(registered["client_name"], "Test client");
    assert!(registered["client_id"].as_str().is_some());

    service.stop().await?;
    Ok(())
}
