use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tokio::sync::RwLock;
// use tokio::time::sleep; // Not needed for now

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConnectionAttempt {
    pub peer_id: String,
    pub attempts: u32,
    pub last_attempt: u64, // Unix timestamp
    pub next_attempt: u64, // Unix timestamp
    pub backoff_duration: Duration,
    pub max_attempts: u32,
    pub connected: bool,
}

impl ConnectionAttempt {
    pub fn new(peer_id: String, max_attempts: u32) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            peer_id,
            attempts: 0,
            last_attempt: now,
            next_attempt: now,
            backoff_duration: Duration::from_secs(1), // Start with 1 second
            max_attempts,
            connected: false,
        }
    }

    pub fn should_retry(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        !self.connected && self.attempts < self.max_attempts && now >= self.next_attempt
    }

    pub fn record_attempt(&mut self, success: bool) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.attempts += 1;
        self.last_attempt = now;

        if success {
            self.connected = true;
            self.backoff_duration = Duration::from_secs(1); // Reset on success
        } else {
            // Exponential backoff with jitter: min(max_duration, current * 2) + random jitter
            self.backoff_duration = std::cmp::min(
                Duration::from_secs(300), // Max 5 minutes
                Duration::from_millis(
                    self.backoff_duration.as_millis() as u64 * 2 + (rand::random::<u64>() % 1000), // Add up to 1 second jitter
                ),
            );
            self.next_attempt = now + self.backoff_duration.as_secs();
        }
    }

    pub fn reset(&mut self) {
        self.attempts = 0;
        self.connected = false;
        self.backoff_duration = Duration::from_secs(1);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.last_attempt = now;
        self.next_attempt = now;
    }

    pub fn is_exhausted(&self) -> bool {
        self.attempts >= self.max_attempts && !self.connected
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConnectionConfig {
    pub max_attempts: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub connection_timeout: Duration,
    pub retry_interval: Duration,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(300), // 5 minutes
            connection_timeout: Duration::from_secs(30),
            retry_interval: Duration::from_secs(60), // Check for retries every minute
        }
    }
}

pub struct ConnectionManager {
    attempts: Arc<RwLock<HashMap<String, ConnectionAttempt>>>,
    config: ConnectionConfig,
    app_handle: AppHandle,
    shutdown_tx: Option<tokio::sync::mpsc::Sender<()>>,
}

impl ConnectionManager {
    pub fn new(app_handle: AppHandle, config: ConnectionConfig) -> Self {
        Self {
            attempts: Arc::new(RwLock::new(HashMap::new())),
            config,
            app_handle,
            shutdown_tx: None,
        }
    }

    pub async fn start_retry_manager(&mut self) -> Result<()> {
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel(1);
        self.shutdown_tx = Some(shutdown_tx);

        let attempts = Arc::clone(&self.attempts);
        let config = self.config.clone();
        let app_handle = self.app_handle.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(config.retry_interval);

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        Self::process_retries(&attempts, &config, &app_handle).await;
                    }
                    _ = shutdown_rx.recv() => {
                        println!("Connection retry manager shutting down");
                        break;
                    }
                }
            }
        });

        Ok(())
    }

    async fn process_retries(
        attempts: &Arc<RwLock<HashMap<String, ConnectionAttempt>>>,
        _config: &ConnectionConfig,
        app_handle: &AppHandle,
    ) {
        let mut retry_peers = Vec::new();

        // Collect peers that should be retried
        {
            let attempts_lock = attempts.read().await;
            for (peer_id, attempt) in attempts_lock.iter() {
                if attempt.should_retry() {
                    retry_peers.push(peer_id.clone());
                }
            }
        }

        // Process retries
        for peer_id in retry_peers {
            println!("Retrying connection to peer: {}", peer_id);

            // Emit retry event to frontend
            if let Err(e) = app_handle.emit(
                "peer_retry_attempt",
                serde_json::json!({
                    "peerId": peer_id,
                    "timestamp": chrono::Utc::now().to_rfc3339()
                }),
            ) {
                eprintln!("Failed to emit retry event: {}", e);
            }

            // Update attempt record
            {
                let mut attempts_lock = attempts.write().await;
                if let Some(attempt) = attempts_lock.get_mut(&peer_id) {
                    attempt.record_attempt(false); // Will be updated if connection succeeds
                }
            }

            // Emit connection attempt to trigger actual connection logic
            if let Err(e) = app_handle.emit(
                "connection_retry_requested",
                serde_json::json!({
                    "peerId": peer_id
                }),
            ) {
                eprintln!("Failed to emit connection retry request: {}", e);
            }
        }

        // Cleanup exhausted attempts
        Self::cleanup_exhausted_attempts(attempts, app_handle).await;
    }

    async fn cleanup_exhausted_attempts(
        attempts: &Arc<RwLock<HashMap<String, ConnectionAttempt>>>,
        app_handle: &AppHandle,
    ) {
        let mut exhausted_peers = Vec::new();

        {
            let mut attempts_lock = attempts.write().await;
            attempts_lock.retain(|peer_id, attempt| {
                if attempt.is_exhausted() {
                    exhausted_peers.push(peer_id.clone());
                    false
                } else {
                    true
                }
            });
        }

        for peer_id in exhausted_peers {
            println!("Giving up on peer after max attempts: {}", peer_id);

            if let Err(e) = app_handle.emit(
                "peer_connection_exhausted",
                serde_json::json!({
                    "peerId": peer_id,
                    "reason": "max_attempts_exceeded"
                }),
            ) {
                eprintln!("Failed to emit connection exhausted event: {}", e);
            }
        }
    }

    pub async fn track_connection_attempt(&self, peer_id: String) -> Result<()> {
        let mut attempts = self.attempts.write().await;
        let attempt = attempts
            .entry(peer_id.clone())
            .or_insert_with(|| ConnectionAttempt::new(peer_id, self.config.max_attempts));

        attempt.record_attempt(false);
        Ok(())
    }

    pub async fn mark_connection_success(&self, peer_id: &str) -> Result<()> {
        let mut attempts = self.attempts.write().await;
        if let Some(attempt) = attempts.get_mut(peer_id) {
            attempt.record_attempt(true);
            println!(
                "Connection to peer {} successful after {} attempts",
                peer_id, attempt.attempts
            );
        }
        Ok(())
    }

    pub async fn mark_connection_failure(&self, peer_id: &str, reason: &str) -> Result<()> {
        let mut attempts = self.attempts.write().await;
        if let Some(attempt) = attempts.get_mut(peer_id) {
            attempt.record_attempt(false);
            println!(
                "Connection to peer {} failed (attempt {}): {}",
                peer_id, attempt.attempts, reason
            );

            // Emit failure event
            self.app_handle.emit(
                "peer_connection_failed",
                serde_json::json!({
                    "peerId": peer_id,
                    "reason": reason,
                    "attempts": attempt.attempts,
                    "maxAttempts": attempt.max_attempts,
                    "nextRetryIn": if attempt.next_attempt > SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() {
                            attempt.next_attempt - SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs()
                        } else { 0 }
                }),
            )?;
        }
        Ok(())
    }

    pub async fn reset_connection_attempts(&self, peer_id: &str) -> Result<()> {
        let mut attempts = self.attempts.write().await;
        if let Some(attempt) = attempts.get_mut(peer_id) {
            attempt.reset();
        }
        Ok(())
    }

    pub async fn remove_peer(&self, peer_id: &str) -> Result<()> {
        let mut attempts = self.attempts.write().await;
        attempts.remove(peer_id);
        Ok(())
    }

    pub async fn get_connection_status(&self, peer_id: &str) -> Option<ConnectionAttempt> {
        let attempts = self.attempts.read().await;
        attempts.get(peer_id).cloned()
    }

    pub async fn get_all_connection_attempts(&self) -> HashMap<String, ConnectionAttempt> {
        let attempts = self.attempts.read().await;
        attempts.clone()
    }

    pub async fn stop_retry_manager(&mut self) -> Result<()> {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(()).await;
        }
        Ok(())
    }

    pub fn update_config(&mut self, config: ConnectionConfig) {
        self.config = config;
    }
}

