use axum::extract::ws::{Message, WebSocket};
use futures::stream::{SplitSink, SplitStream};
use futures::{Sink, Stream, StreamExt};
use samod::{ConnDirection, Repo};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio_tungstenite::tungstenite;

struct WebSocketAdapter {
    sink: SplitSink<WebSocket, Message>,
    stream: SplitStream<WebSocket>,
}

impl Stream for WebSocketAdapter {
    type Item = Result<tungstenite::Message, tungstenite::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.stream).poll_next(cx) {
            Poll::Ready(Some(Ok(msg))) => {
                let tungstenite_msg = match msg {
                    Message::Binary(data) => tungstenite::Message::Binary(data),
                    Message::Text(text) => tungstenite::Message::Text(text.to_string().into()),
                    Message::Close(frame) => {
                        let close_frame = frame.map(|f| tungstenite::protocol::CloseFrame {
                            code: tungstenite::protocol::frame::coding::CloseCode::from(f.code),
                            reason: f.reason.to_string().into(),
                        });
                        tungstenite::Message::Close(close_frame)
                    }
                    Message::Ping(data) => tungstenite::Message::Ping(data),
                    Message::Pong(data) => tungstenite::Message::Pong(data),
                };
                Poll::Ready(Some(Ok(tungstenite_msg)))
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(tungstenite::Error::Io(
                std::io::Error::other(e.to_string()),
            )))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Sink<tungstenite::Message> for WebSocketAdapter {
    type Error = tungstenite::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.sink)
            .poll_ready(cx)
            .map_err(|e| tungstenite::Error::Io(std::io::Error::other(e.to_string())))
    }

    fn start_send(mut self: Pin<&mut Self>, item: tungstenite::Message) -> Result<(), Self::Error> {
        let axum_msg = match item {
            tungstenite::Message::Binary(data) => Message::Binary(data),
            tungstenite::Message::Text(text) => Message::Text(text.to_string().into()),
            tungstenite::Message::Close(frame) => {
                let axum_frame = frame.map(|f| axum::extract::ws::CloseFrame {
                    code: f.code.into(),
                    reason: f.reason.to_string().into(),
                });
                Message::Close(axum_frame)
            }
            tungstenite::Message::Ping(data) => Message::Ping(data),
            tungstenite::Message::Pong(data) => Message::Pong(data),
            tungstenite::Message::Frame(_) => {
                return Err(tungstenite::Error::Io(std::io::Error::other(
                    "Raw frames not supported",
                )));
            }
        };
        Pin::new(&mut self.sink)
            .start_send(axum_msg)
            .map_err(|e| tungstenite::Error::Io(std::io::Error::other(e.to_string())))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.sink)
            .poll_flush(cx)
            .map_err(|e| tungstenite::Error::Io(std::io::Error::other(e.to_string())))
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.sink)
            .poll_close(cx)
            .map_err(|e| tungstenite::Error::Io(std::io::Error::other(e.to_string())))
    }
}

pub async fn handle_websocket_connection(
    axum_socket: WebSocket,
    repo: Arc<Repo>,
    connection_count: Arc<AtomicUsize>,
) {
    let connection_id = uuid::Uuid::new_v4();
    connection_count.fetch_add(1, Ordering::Relaxed);
    let count = connection_count.load(Ordering::Relaxed);
    tracing::info!(
        "[{}] WebSocket connected. Total connections: {}",
        connection_id,
        count
    );

    let (sink, stream) = axum_socket.split();
    let adapter = WebSocketAdapter { sink, stream };

    tracing::debug!("[{}] Starting samod connection", connection_id);
    let finish_reason = repo
        .connect_tungstenite(adapter, ConnDirection::Incoming)
        .await;

    tracing::info!(
        "[{}] Connection finished with reason: {:?}",
        connection_id,
        finish_reason
    );

    connection_count.fetch_sub(1, Ordering::Relaxed);
    let count = connection_count.load(Ordering::Relaxed);
    tracing::info!(
        "[{}] WebSocket disconnected. Total connections: {}",
        connection_id,
        count
    );
}
