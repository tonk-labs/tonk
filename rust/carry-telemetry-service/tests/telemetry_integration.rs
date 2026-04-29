//! Integration tests for the carry telemetry service.
//!
//! These tests run a local HTTP server that mirrors the Cloudflare Worker
//! behavior, exercising the full request/response cycle.

use carry_telemetry_service::helpers::TelemetryTestServer;
use serde_json::json;

// ══════════════════════════════════════════════════════════════════════════════
// Health check
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn health_returns_ok() {
    let server = TelemetryTestServer::start().await.unwrap();
    let resp = reqwest::get(format!("{}/health", server.endpoint))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "OK");
    server.stop().await;
}

// ══════════════════════════════════════════════════════════════════════════════
// POST /ping -- valid payloads
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn ping_valid_payload_returns_ok() {
    let server = TelemetryTestServer::start().await.unwrap();
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/ping", server.endpoint))
        .json(&json!({
            "id": "abcdef0123456789",
            "command": "query",
            "version": "0.1.0"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let pings = server.recorded_pings().await;
    assert_eq!(pings.len(), 1);
    assert_eq!(pings[0].id, "abcdef0123456789");
    assert_eq!(pings[0].command, "query");
    assert_eq!(pings[0].version, "0.1.0");
    server.stop().await;
}

#[tokio::test]
async fn ping_records_multiple_events() {
    let server = TelemetryTestServer::start().await.unwrap();
    let client = reqwest::Client::new();

    let commands = ["init", "query", "assert", "retract", "status"];
    for cmd in &commands {
        client
            .post(format!("{}/ping", server.endpoint))
            .json(&json!({
                "id": "aabbccdd11223344",
                "command": cmd,
                "version": "0.1.0"
            }))
            .send()
            .await
            .unwrap();
    }

    let pings = server.recorded_pings().await;
    assert_eq!(pings.len(), commands.len());
    for (i, cmd) in commands.iter().enumerate() {
        assert_eq!(pings[i].command, *cmd);
    }
    server.stop().await;
}

#[tokio::test]
async fn ping_distinct_user_ids() {
    let server = TelemetryTestServer::start().await.unwrap();
    let client = reqwest::Client::new();

    let ids = ["aaaa000000000000", "bbbb111111111111", "cccc222222222222"];
    for id in &ids {
        client
            .post(format!("{}/ping", server.endpoint))
            .json(&json!({
                "id": id,
                "command": "init",
                "version": "0.1.0"
            }))
            .send()
            .await
            .unwrap();
    }

    let pings = server.recorded_pings().await;
    assert_eq!(pings.len(), 3);
    let unique_ids: std::collections::HashSet<&str> = pings.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(unique_ids.len(), 3);
    server.stop().await;
}

#[tokio::test]
async fn ping_minimum_length_fields() {
    let server = TelemetryTestServer::start().await.unwrap();
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/ping", server.endpoint))
        .json(&json!({
            "id": "a",
            "command": "q",
            "version": "0"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let pings = server.recorded_pings().await;
    assert_eq!(pings.len(), 1);
    server.stop().await;
}

#[tokio::test]
async fn ping_maximum_length_id() {
    let server = TelemetryTestServer::start().await.unwrap();
    let client = reqwest::Client::new();
    // 32 hex chars (maximum)
    let resp = client
        .post(format!("{}/ping", server.endpoint))
        .json(&json!({
            "id": "abcdef0123456789abcdef0123456789",
            "command": "init",
            "version": "0.1.0"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    server.stop().await;
}

// ══════════════════════════════════════════════════════════════════════════════
// POST /ping -- invalid payloads
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn ping_empty_id_rejected() {
    let server = TelemetryTestServer::start().await.unwrap();
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/ping", server.endpoint))
        .json(&json!({
            "id": "",
            "command": "query",
            "version": "0.1.0"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    assert_eq!(server.recorded_pings().await.len(), 0);
    server.stop().await;
}

#[tokio::test]
async fn ping_non_hex_id_rejected() {
    let server = TelemetryTestServer::start().await.unwrap();
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/ping", server.endpoint))
        .json(&json!({
            "id": "not-hex-at-all!!",
            "command": "query",
            "version": "0.1.0"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    assert_eq!(server.recorded_pings().await.len(), 0);
    server.stop().await;
}

#[tokio::test]
async fn ping_id_too_long_rejected() {
    let server = TelemetryTestServer::start().await.unwrap();
    let client = reqwest::Client::new();
    // 33 hex chars (one over max)
    let resp = client
        .post(format!("{}/ping", server.endpoint))
        .json(&json!({
            "id": "abcdef0123456789abcdef01234567890",
            "command": "query",
            "version": "0.1.0"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    server.stop().await;
}

#[tokio::test]
async fn ping_empty_command_rejected() {
    let server = TelemetryTestServer::start().await.unwrap();
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/ping", server.endpoint))
        .json(&json!({
            "id": "abcdef0123456789",
            "command": "",
            "version": "0.1.0"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    server.stop().await;
}

#[tokio::test]
async fn ping_empty_version_rejected() {
    let server = TelemetryTestServer::start().await.unwrap();
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/ping", server.endpoint))
        .json(&json!({
            "id": "abcdef0123456789",
            "command": "query",
            "version": ""
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    server.stop().await;
}

#[tokio::test]
async fn ping_command_too_long_rejected() {
    let server = TelemetryTestServer::start().await.unwrap();
    let client = reqwest::Client::new();
    let long_command = "x".repeat(65);
    let resp = client
        .post(format!("{}/ping", server.endpoint))
        .json(&json!({
            "id": "abcdef0123456789",
            "command": long_command,
            "version": "0.1.0"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    server.stop().await;
}

#[tokio::test]
async fn ping_version_too_long_rejected() {
    let server = TelemetryTestServer::start().await.unwrap();
    let client = reqwest::Client::new();
    let long_version = "9".repeat(33);
    let resp = client
        .post(format!("{}/ping", server.endpoint))
        .json(&json!({
            "id": "abcdef0123456789",
            "command": "query",
            "version": long_version,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    server.stop().await;
}

#[tokio::test]
async fn ping_missing_fields_rejected() {
    let server = TelemetryTestServer::start().await.unwrap();
    let client = reqwest::Client::new();

    // Missing id
    let resp = client
        .post(format!("{}/ping", server.endpoint))
        .json(&json!({
            "command": "query",
            "version": "0.1.0"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    // Missing command
    let resp = client
        .post(format!("{}/ping", server.endpoint))
        .json(&json!({
            "id": "abcdef0123456789",
            "version": "0.1.0"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    // Missing version
    let resp = client
        .post(format!("{}/ping", server.endpoint))
        .json(&json!({
            "id": "abcdef0123456789",
            "command": "query"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    assert_eq!(server.recorded_pings().await.len(), 0);
    server.stop().await;
}

#[tokio::test]
async fn ping_invalid_json_rejected() {
    let server = TelemetryTestServer::start().await.unwrap();
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/ping", server.endpoint))
        .header("content-type", "application/json")
        .body("not json at all")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    server.stop().await;
}

#[tokio::test]
async fn ping_empty_body_rejected() {
    let server = TelemetryTestServer::start().await.unwrap();
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/ping", server.endpoint))
        .header("content-type", "application/json")
        .body("")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    server.stop().await;
}

// ══════════════════════════════════════════════════════════════════════════════
// CORS preflight
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn options_ping_returns_cors_headers() {
    let server = TelemetryTestServer::start().await.unwrap();
    let client = reqwest::Client::new();
    let resp = client
        .request(
            reqwest::Method::OPTIONS,
            format!("{}/ping", server.endpoint),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
    assert_eq!(
        resp.headers()
            .get("access-control-allow-origin")
            .unwrap()
            .to_str()
            .unwrap(),
        "*"
    );
    assert!(
        resp.headers()
            .get("access-control-allow-methods")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("POST")
    );
    server.stop().await;
}

#[tokio::test]
async fn post_ping_response_has_cors_header() {
    let server = TelemetryTestServer::start().await.unwrap();
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/ping", server.endpoint))
        .json(&json!({
            "id": "abcdef0123456789",
            "command": "query",
            "version": "0.1.0"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()
            .get("access-control-allow-origin")
            .unwrap()
            .to_str()
            .unwrap(),
        "*"
    );
    server.stop().await;
}

// ══════════════════════════════════════════════════════════════════════════════
// Wrong methods / routes
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn get_ping_returns_404() {
    let server = TelemetryTestServer::start().await.unwrap();
    let resp = reqwest::get(format!("{}/ping", server.endpoint))
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    server.stop().await;
}

#[tokio::test]
async fn unknown_route_returns_404() {
    let server = TelemetryTestServer::start().await.unwrap();
    let resp = reqwest::get(format!("{}/nonexistent", server.endpoint))
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    server.stop().await;
}

// ══════════════════════════════════════════════════════════════════════════════
// Server lifecycle
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn clear_resets_recorded_pings() {
    let server = TelemetryTestServer::start().await.unwrap();
    let client = reqwest::Client::new();

    client
        .post(format!("{}/ping", server.endpoint))
        .json(&json!({
            "id": "abcdef0123456789",
            "command": "init",
            "version": "0.1.0"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(server.recorded_pings().await.len(), 1);
    server.clear().await;
    assert_eq!(server.recorded_pings().await.len(), 0);
    server.stop().await;
}
