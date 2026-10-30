//! The OAuth dance against the local server, over HTTP: what an MCP
//! client does (discovery, registration, `/authorize`, `/token`) with a
//! stand-in for the approval page in between, which posts a token the
//! way the real page will.
//!
//! The protocol's refusals are tested beside it, in
//! `src/oauth/tests.rs`; this checks the wire: headers, cookies, and
//! that the native server routes what the worker routes.

#![cfg(all(feature = "helpers", not(target_arch = "wasm32")))]

use serde_json::{Value, json};
use tonk_access_service::helpers::{AccessServiceSettings, access_service};

/// RFC 7636 appendix B.
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
const CALLBACK: &str = "http://127.0.0.1:41234/callback";

#[dialog_common::test]
async fn it_runs_the_authorization_code_flow() -> anyhow::Result<()> {
    let service = access_service(AccessServiceSettings::default()).await?;
    let origin = service
        .address
        .access_service_url
        .trim_end_matches('/')
        .to_owned();
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;

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
    let registered: Value = client
        .post(server["registration_endpoint"].as_str().unwrap())
        .json(&json!({
            "client_name": "Test client",
            "redirect_uris": ["http://127.0.0.1/callback"],
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let client_id = registered["client_id"].as_str().unwrap();

    // The browser opens `/authorize` and is handed to the page.
    let authorize = client
        .get(server["authorization_endpoint"].as_str().unwrap())
        .query(&[
            ("response_type", "code"),
            ("client_id", client_id),
            ("redirect_uri", CALLBACK),
            ("code_challenge", CHALLENGE),
            ("code_challenge_method", "S256"),
            ("resource", &format!("{origin}/mcp")),
            ("state", "opaque"),
        ])
        .send()
        .await?;
    assert_eq!(authorize.status(), 303);
    let location = url::Url::parse(authorize.headers()["location"].to_str()?)?;
    assert_eq!(location.path(), "/settings/agent");
    let handle = location
        .query_pairs()
        .find(|(key, _)| key == "request")
        .map(|(_, value)| value.into_owned())
        .unwrap();
    let cookie = authorize.headers()["set-cookie"]
        .to_str()?
        .split(';')
        .next()
        .unwrap()
        .to_owned();

    // The page reads the request and answers it with a token.
    let described: Value = client
        .get(format!("{origin}/authorize/{handle}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(described["client_name"], "Test client");
    assert_eq!(described["redirect_host"], "127.0.0.1");
    let approved: Value = client
        .post(format!("{origin}/authorize/{handle}"))
        .header("Origin", &origin)
        .header("Cookie", &cookie)
        .json(&json!({ "token": "dG9rZW4" }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let redirect = url::Url::parse(approved["redirect"].as_str().unwrap())?;
    assert!(redirect.as_str().starts_with(CALLBACK));
    let code = redirect
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .unwrap();

    // The client redeems the code, once.
    let form = [
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("client_id", client_id),
        ("redirect_uri", CALLBACK),
        ("code_verifier", VERIFIER),
        ("resource", &format!("{origin}/mcp")),
    ];
    let token = client
        .post(server["token_endpoint"].as_str().unwrap())
        .form(&form)
        .send()
        .await?;
    assert_eq!(token.headers()["cache-control"], "no-store");
    let token: Value = token.error_for_status()?.json().await?;
    assert_eq!(
        token,
        json!({ "access_token": "dG9rZW4", "token_type": "Bearer" })
    );
    let replayed = client
        .post(server["token_endpoint"].as_str().unwrap())
        .form(&form)
        .send()
        .await?;
    assert_eq!(replayed.status(), 400);

    service.stop().await?;
    Ok(())
}
