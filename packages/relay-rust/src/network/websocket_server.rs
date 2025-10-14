use crate::error::{RelayError, Result};
use samod::{ConnDirection, Repo};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};

pub struct WebSocketServer {
    repo: Arc<Repo>,
    listener: TcpListener,
    connection_count: Arc<AtomicUsize>,
}

impl WebSocketServer {
    pub async fn new(
        repo: Arc<Repo>,
        addr: SocketAddr,
        connection_count: Arc<AtomicUsize>,
    ) -> Result<Self> {
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| RelayError::WebSocket(format!("Failed to bind to {}: {}", addr, e)))?;

        tracing::info!("WebSocket server listening on {}", addr);

        Ok(Self {
            repo,
            listener,
            connection_count,
        })
    }

    pub async fn run(self) -> Result<()> {
        loop {
            match self.listener.accept().await {
                Ok((stream, addr)) => {
                    let repo = Arc::clone(&self.repo);
                    let connection_count = Arc::clone(&self.connection_count);

                    tokio::spawn(async move {
                        if let Err(e) =
                            Self::handle_connection(repo, stream, addr, connection_count).await
                        {
                            tracing::error!("WebSocket connection error from {}: {}", addr, e);
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("Failed to accept connection: {}", e);
                }
            }
        }
    }

    async fn handle_connection(
        repo: Arc<Repo>,
        stream: TcpStream,
        addr: SocketAddr,
        connection_count: Arc<AtomicUsize>,
    ) -> Result<()> {
        connection_count.fetch_add(1, Ordering::Relaxed);
        let count = connection_count.load(Ordering::Relaxed);
        tracing::info!(
            "WebSocket connected from {}. Total connections: {}",
            addr,
            count
        );

        let ws_stream = tokio_tungstenite::accept_async(stream)
            .await
            .map_err(|e| RelayError::WebSocket(format!("Failed to accept WebSocket: {}", e)))?;

        let finish_reason = repo
            .connect_tungstenite(ws_stream, ConnDirection::Incoming)
            .await;

        tracing::debug!("Connection finished with reason: {:?}", finish_reason);

        connection_count.fetch_sub(1, Ordering::Relaxed);
        let count = connection_count.load(Ordering::Relaxed);
        tracing::info!(
            "WebSocket disconnected from {}. Total connections: {}",
            addr,
            count
        );

        Ok(())
    }
}
