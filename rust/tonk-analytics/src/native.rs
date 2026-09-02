//! Native capture client for the CLI: queue in memory, POST one
//! `/batch` request on flush, never block the command for more than
//! the caller's timeout, never surface an error.

use std::time::Duration;

use serde::Serialize;
use serde_json::{Map, Value};

/// One queued event, in PostHog `/batch` wire shape.
#[derive(Debug, Clone, Serialize)]
pub struct Event {
    /// Event name — one of [`crate::event`].
    pub event: String,
    /// Hashed identity ([`crate::distinct_id`]) or `"tonk:anonymous"`.
    pub distinct_id: String,
    /// Content-free properties.
    pub properties: Map<String, Value>,
}

struct Config {
    host: String,
    api_key: String,
    distinct_id: String,
}

/// Batch capture client. A disabled client (no key, opt-out) is a
/// zero-cost no-op: `capture` drops events, `flush` returns without
/// touching the network.
pub struct Client {
    config: Option<Config>,
    events: Vec<Event>,
}

impl Client {
    /// A client that ignores everything.
    pub fn disabled() -> Self {
        Self {
            config: None,
            events: Vec::new(),
        }
    }

    /// A client with explicit config — used by tests and by
    /// [`Client::from_env`].
    pub fn new(host: String, api_key: String, distinct_id: String) -> Self {
        Self {
            config: Some(Config {
                host,
                api_key,
                distinct_id,
            }),
            events: Vec::new(),
        }
    }

    /// Resolve key/host/opt-out from the environment. `enabled` is the
    /// caller's own switch (the CLI's persisted setting); env opt-outs
    /// and a missing API key also disable.
    pub fn from_env(distinct_id: String, enabled: bool) -> Self {
        if !enabled || crate::env_opt_out(|key| std::env::var(key).ok()) {
            return Self::disabled();
        }
        match crate::api_key() {
            Some(api_key) => Self::new(crate::host(), api_key, distinct_id),
            None => Self::disabled(),
        }
    }

    /// Whether this client will actually send anything.
    pub fn is_enabled(&self) -> bool {
        self.config.is_some()
    }

    /// Queue one event. Adds the standard context properties
    /// (`$lib`, `version`, `os`, `arch`).
    pub fn capture(&mut self, event: &str, mut properties: Map<String, Value>) {
        let Some(config) = &self.config else { return };
        properties.insert("$lib".to_owned(), Value::from("tonk-analytics"));
        properties.insert("version".to_owned(), Value::from(env!("CARGO_PKG_VERSION")));
        properties.insert("os".to_owned(), Value::from(std::env::consts::OS));
        properties.insert("arch".to_owned(), Value::from(std::env::consts::ARCH));
        self.events.push(Event {
            event: event.to_owned(),
            distinct_id: config.distinct_id.clone(),
            properties,
        });
    }

    /// Validate and queue one canonical account event. Transport context is
    /// added here so account call sites cannot vary its shape.
    pub fn capture_account(
        &mut self,
        event: &crate::account::AccountEvent,
    ) -> Result<(), crate::account::ValidationError> {
        let mut properties = event.validated_properties()?;
        properties.insert("environment".to_owned(), Value::from("cli"));
        self.capture(crate::event::ACCOUNT, properties);
        Ok(())
    }

    /// The `/batch` request body, or `None` when disabled or empty.
    /// Public so tests can assert the wire shape without a server.
    pub fn payload(&self) -> Option<Value> {
        let config = self.config.as_ref()?;
        if self.events.is_empty() {
            return None;
        }
        Some(serde_json::json!({
            "api_key": config.api_key,
            "batch": self.events,
        }))
    }

    /// Send queued events, best-effort within `timeout`. Network
    /// errors and timeouts are swallowed: telemetry must never fail
    /// or slow a command beyond the cap.
    pub async fn flush(self, timeout: Duration) {
        let Some(payload) = self.payload() else {
            return;
        };
        let Some(config) = self.config.as_ref() else {
            return;
        };
        let url = format!("{}/batch/", config.host.trim_end_matches('/'));
        let send = reqwest::Client::new().post(url).json(&payload).send();
        let _ = tokio::time::timeout(timeout, send).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[dialog_common::test]
    fn disabled_client_queues_nothing() {
        let mut client = Client::disabled();
        client.capture(crate::event::CLI_COMMAND_RUN, Map::new());
        assert!(!client.is_enabled());
        assert!(client.payload().is_none());
    }

    #[dialog_common::test]
    fn payload_has_batch_wire_shape() {
        let mut client = Client::new(
            "http://localhost:1".to_owned(),
            "key".to_owned(),
            "tonk:abc".to_owned(),
        );
        let mut props = Map::new();
        props.insert("command".to_owned(), Value::from("eval"));
        client.capture(crate::event::CLI_COMMAND_RUN, props);
        let payload = client.payload().expect("payload");
        assert_eq!(payload["api_key"], "key");
        let event = &payload["batch"][0];
        assert_eq!(event["event"], "cli_command_run");
        assert_eq!(event["distinct_id"], "tonk:abc");
        assert_eq!(event["properties"]["command"], "eval");
        assert_eq!(event["properties"]["os"], std::env::consts::OS);
        assert!(event["properties"]["version"].is_string());
    }

    #[dialog_common::test]
    fn account_capture_has_the_validated_shape_and_cli_context() {
        use crate::account::{
            AccountAction, AccountEvent, AccountState, Journey, Stage, Surface, Trigger,
        };
        let mut client = Client::new("http://localhost:1".into(), "key".into(), "tonk:abc".into());
        let event = AccountEvent::started(
            Journey::Login,
            AccountAction::Login,
            Stage::Input,
            Surface::NativeCli,
            Trigger::User,
            AccountState::None,
            "opaque-1",
        );
        client.capture_account(&event).unwrap();
        let event = &client.payload().unwrap()["batch"][0];
        assert_eq!(event["event"], "account_event");
        assert_eq!(event["properties"]["environment"], "cli");
        assert_eq!(event["properties"]["action"], "login");
        assert_eq!(event["properties"]["version"], env!("CARGO_PKG_VERSION"));
    }

    /// Minimal one-shot HTTP server: accept one connection, read one
    /// request (headers + content-length body), respond 200, return
    /// the raw request text.
    async fn serve_once(listener: TcpListener) -> String {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = stream.read(&mut chunk).await.expect("read");
            if n == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..n]);
            let text = String::from_utf8_lossy(&buffer).into_owned();
            if let Some(split) = text.find("\r\n\r\n") {
                let length: usize = text[..split]
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse().unwrap_or(0))
                    })
                    .unwrap_or(0);
                if text[split + 4..].len() >= length {
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
                        )
                        .await
                        .expect("write");
                    return text;
                }
            }
        }
        String::from_utf8_lossy(&buffer).into_owned()
    }

    #[dialog_common::test]
    async fn flush_posts_batch_to_endpoint() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = tokio::spawn(serve_once(listener));

        let mut client = Client::new(
            format!("http://{addr}"),
            "test-key".to_owned(),
            "tonk:abc".to_owned(),
        );
        client.capture(crate::event::CLI_COMMAND_RUN, Map::new());
        client.flush(Duration::from_secs(5)).await;

        let request = server.await.expect("server task");
        assert!(request.starts_with("POST /batch/"));
        assert!(request.contains("\"cli_command_run\""));
        assert!(request.contains("\"tonk:abc\""));
        assert!(request.contains("\"test-key\""));
    }

    #[dialog_common::test]
    async fn flush_times_out_quietly_when_endpoint_is_dead() {
        // Nothing listens on this port; flush must return, not hang.
        let mut client = Client::new(
            "http://127.0.0.1:1".to_owned(),
            "test-key".to_owned(),
            "tonk:abc".to_owned(),
        );
        client.capture(crate::event::CLI_COMMAND_RUN, Map::new());
        client.flush(Duration::from_millis(300)).await;
    }

    #[dialog_common::test]
    fn from_env_gates_on_enabled_flag_key_and_opt_out() {
        // SAFETY: process-global env mutation; this is the only test
        // in the crate that touches these variables, and the other
        // tests construct clients via `Client::new` without reading
        // the environment, so parallel test threads don't observe it.
        unsafe {
            std::env::remove_var("DO_NOT_TRACK");
            std::env::remove_var("TONK_TELEMETRY");
            std::env::remove_var("TONK_POSTHOG_KEY");
            std::env::remove_var("TONK_POSTHOG_ENDPOINT");
            std::env::remove_var("TONK_POSTHOG_HOST");
        }
        // No runtime key (and test builds bake none in) => disabled.
        assert!(!Client::from_env("tonk:abc".to_owned(), true).is_enabled());

        unsafe { std::env::set_var("TONK_POSTHOG_KEY", "test-key") };
        // Caller's own switch wins even with a key present.
        assert!(!Client::from_env("tonk:abc".to_owned(), false).is_enabled());
        assert!(Client::from_env("tonk:abc".to_owned(), true).is_enabled());

        unsafe { std::env::set_var("DO_NOT_TRACK", "1") };
        assert!(!Client::from_env("tonk:abc".to_owned(), true).is_enabled());

        unsafe {
            std::env::remove_var("DO_NOT_TRACK");
            std::env::set_var("TONK_TELEMETRY", "off");
        }
        assert!(!Client::from_env("tonk:abc".to_owned(), true).is_enabled());

        unsafe {
            std::env::remove_var("TONK_TELEMETRY");
            std::env::remove_var("TONK_POSTHOG_KEY");
        }
    }
}
