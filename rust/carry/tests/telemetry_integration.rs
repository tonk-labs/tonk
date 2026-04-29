//! Integration tests for the carry telemetry client.
//!
//! Uses the carry-telemetry-service test server to verify the full
//! client-server round trip without any external dependencies.

use carry_telemetry_service::helpers::TelemetryTestServer;

// ══════════════════════════════════════════════════════════════════════════════
// Blinded ID
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn blinded_id_is_deterministic() {
    let did = "did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD";
    assert_eq!(
        carry::telemetry::blinded_id(did),
        carry::telemetry::blinded_id(did)
    );
}

#[test]
fn blinded_id_is_hex_and_16_chars() {
    let id =
        carry::telemetry::blinded_id("did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD");
    assert_eq!(id.len(), 16);
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn different_dids_produce_different_ids() {
    let id1 =
        carry::telemetry::blinded_id("did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD");
    let id2 =
        carry::telemetry::blinded_id("did:key:z6Mkf5rGMoatrSj1f4CyvuHqdjKN6pVpGGqruHMgfJBuRnQE");
    assert_ne!(id1, id2);
}

// ══════════════════════════════════════════════════════════════════════════════
// Client ping -> test server round trip
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn ping_reaches_server() {
    let server = TelemetryTestServer::start().await.unwrap();
    let ping_url = format!("{}/ping", server.endpoint);

    let did = "did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD";
    let handle = carry::telemetry::ping_to(&ping_url, did, "query");
    let _ = handle.await;

    let pings = server.recorded_pings().await;
    assert_eq!(pings.len(), 1);
    assert_eq!(pings[0].id, carry::telemetry::blinded_id(did));
    assert_eq!(pings[0].command, "query");
    assert_eq!(pings[0].version, env!("CARGO_PKG_VERSION"));

    server.stop().await;
}

#[tokio::test]
async fn ping_sends_correct_blinded_id() {
    let server = TelemetryTestServer::start().await.unwrap();
    let ping_url = format!("{}/ping", server.endpoint);

    let did1 = "did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD";
    let did2 = "did:key:z6Mkf5rGMoatrSj1f4CyvuHqdjKN6pVpGGqruHMgfJBuRnQE";

    let h1 = carry::telemetry::ping_to(&ping_url, did1, "init");
    let h2 = carry::telemetry::ping_to(&ping_url, did2, "query");
    let _ = tokio::join!(h1, h2);

    let pings = server.recorded_pings().await;
    assert_eq!(pings.len(), 2);

    let expected_id1 = carry::telemetry::blinded_id(did1);
    let expected_id2 = carry::telemetry::blinded_id(did2);
    assert_ne!(expected_id1, expected_id2);

    // Pings may arrive in any order
    let ids: std::collections::HashSet<String> = pings.iter().map(|p| p.id.clone()).collect();
    assert!(ids.contains(&expected_id1));
    assert!(ids.contains(&expected_id2));

    server.stop().await;
}

#[tokio::test]
async fn ping_does_not_contain_raw_did() {
    let server = TelemetryTestServer::start().await.unwrap();
    let ping_url = format!("{}/ping", server.endpoint);

    let did = "did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD";
    let handle = carry::telemetry::ping_to(&ping_url, did, "status");
    let _ = handle.await;

    let pings = server.recorded_pings().await;
    assert_eq!(pings.len(), 1);

    // The raw DID must not appear anywhere in the recorded data
    assert!(!pings[0].id.contains("did:key"));
    assert!(!pings[0].id.contains("z6Mk"));
    assert!(!pings[0].command.contains("did:key"));
    assert!(!pings[0].version.contains("did:key"));

    server.stop().await;
}

#[tokio::test]
async fn ping_sends_all_command_types() {
    let server = TelemetryTestServer::start().await.unwrap();
    let ping_url = format!("{}/ping", server.endpoint);
    let did = "did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD";

    let commands = [
        "init", "query", "assert", "retract", "status", "identity", "invite", "join",
    ];
    let mut handles = Vec::new();
    for cmd in &commands {
        handles.push(carry::telemetry::ping_to(&ping_url, did, cmd));
    }
    for h in handles {
        let _ = h.await;
    }

    let pings = server.recorded_pings().await;
    assert_eq!(pings.len(), commands.len());

    let recorded_commands: std::collections::HashSet<String> =
        pings.iter().map(|p| p.command.clone()).collect();
    for cmd in &commands {
        assert!(
            recorded_commands.contains(*cmd),
            "Command '{}' should be recorded",
            cmd
        );
    }

    server.stop().await;
}

#[tokio::test]
async fn ping_silently_fails_on_unreachable_server() {
    // Point at a server that doesn't exist -- should not panic or error
    let handle = carry::telemetry::ping_to(
        "http://127.0.0.1:1/ping",
        "did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD",
        "query",
    );

    // Just verify it doesn't panic; the spawned task will fail silently
    let _ = handle.await;
}

#[tokio::test]
async fn ping_includes_version() {
    let server = TelemetryTestServer::start().await.unwrap();
    let ping_url = format!("{}/ping", server.endpoint);

    let handle = carry::telemetry::ping_to(
        &ping_url,
        "did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD",
        "init",
    );
    let _ = handle.await;

    let pings = server.recorded_pings().await;
    assert_eq!(pings.len(), 1);
    assert!(!pings[0].version.is_empty(), "Version should not be empty");
    // Version should look like a semver string
    assert!(
        pings[0].version.contains('.'),
        "Version '{}' should be semver-like",
        pings[0].version
    );

    server.stop().await;
}

#[tokio::test]
async fn repeated_pings_same_did_produce_same_id() {
    let server = TelemetryTestServer::start().await.unwrap();
    let ping_url = format!("{}/ping", server.endpoint);
    let did = "did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD";

    let mut handles = Vec::new();
    for _ in 0..5 {
        handles.push(carry::telemetry::ping_to(&ping_url, did, "query"));
    }
    for h in handles {
        let _ = h.await;
    }

    let pings = server.recorded_pings().await;
    assert_eq!(pings.len(), 5);

    let expected_id = carry::telemetry::blinded_id(did);
    for ping in &pings {
        assert_eq!(
            ping.id, expected_id,
            "All pings from same DID should have same blinded ID"
        );
    }

    server.stop().await;
}
