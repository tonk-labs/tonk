//! Minimal, privacy-preserving usage telemetry.
//!
//! Sends an anonymous ping on each CLI invocation so we can count unique
//! users and see which commands are used. The user's DID is hashed
//! client-side (blake3 with a fixed salt) so neither the DID nor IP
//! address reaches us in identifiable form.
//!
//! Opt out by setting the environment variable `DO_NOT_TRACK=1`.

const DEFAULT_TELEMETRY_URL: &str = "https://carry-telemetry-service.tonk.workers.dev/ping";
const SALT: &str = "carry-telemetry-v1";

/// Resolve the telemetry endpoint. Reads `CARRY_TELEMETRY_URL` for testing,
/// falls back to the production URL.
fn telemetry_url() -> String {
    std::env::var("CARRY_TELEMETRY_URL").unwrap_or_else(|_| DEFAULT_TELEMETRY_URL.to_string())
}

/// Derive a blinded, non-reversible user identifier from a DID.
pub fn blinded_id(did: &str) -> String {
    let hash = blake3::hash(format!("{SALT}:{did}").as_bytes());
    hash.to_hex()[..16].to_string()
}

/// Fire-and-forget a telemetry ping. Returns a handle that should be
/// awaited before the process exits (otherwise tokio may cancel it).
/// Respects `DO_NOT_TRACK=1`.
pub fn ping(did: &str, command: &str) -> Option<tokio::task::JoinHandle<()>> {
    if std::env::var("DO_NOT_TRACK").unwrap_or_default() == "1" {
        return None;
    }
    Some(ping_to(&telemetry_url(), did, command))
}

/// Fire-and-forget a telemetry ping to a specific URL.
/// Exposed for testing; production code should use `ping()`.
pub fn ping_to(url: &str, did: &str, command: &str) -> tokio::task::JoinHandle<()> {
    let url = url.to_string();
    let id = blinded_id(did);
    let version = env!("CARGO_PKG_VERSION").to_string();
    let command = command.to_string();

    tokio::spawn(async move {
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            send_ping(&url, &id, &command, &version),
        )
        .await;
    })
}

async fn send_ping(
    url: &str,
    id: &str,
    command: &str,
    version: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::new();
    client
        .post(url)
        .json(&serde_json::json!({
            "id": id,
            "command": command,
            "version": version,
        }))
        .send()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blinded_id_is_deterministic() {
        let did = "did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD";
        let id1 = blinded_id(did);
        let id2 = blinded_id(did);
        assert_eq!(id1, id2);
    }

    #[test]
    fn blinded_id_is_16_hex_chars() {
        let id = blinded_id("did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD");
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn blinded_id_differs_for_different_dids() {
        let id1 = blinded_id("did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD");
        let id2 = blinded_id("did:key:z6Mkf5rGMoatrSj1f4CyvuHqdjKN6pVpGGqruHMgfJBuRnQE");
        assert_ne!(id1, id2);
    }

    #[test]
    fn blinded_id_does_not_contain_did() {
        let did = "did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD";
        let id = blinded_id(did);
        assert!(!did.contains(&id));
        assert!(!id.contains("did:key"));
    }
}
