use crate::error::{Result, VfsError};
use std::sync::Arc;

#[cfg(target_arch = "wasm32")]
pub mod wasm_impl {
    use super::*;
    use futures::{
        channel::{mpsc, oneshot},
        Sink, SinkExt, Stream, StreamExt,
    };
    use js_sys::Uint8Array;
    use samod::{ConnDirection, Repo};
    use std::cell::RefCell;
    use std::pin::Pin;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::task::{Context, Poll};
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;
    use web_sys::{BinaryType, CloseEvent, ErrorEvent, MessageEvent, WebSocket};

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = console)]
        fn log(s: &str);
    }

    // Global counters for debugging
    static CONNECTION_COUNTER: AtomicU32 = AtomicU32::new(0);
    static SENDER_COUNTER: AtomicU32 = AtomicU32::new(0);

    // WebSocket wrapper that converts web_sys WebSocket to a Stream/Sink
    pub struct WasmWebSocket {
        ws: WebSocket,
        _closures: ClosureHandlers,
        receiver: mpsc::UnboundedReceiver<WsMessage>,
        connection_id: u32,
    }

    // Hold closures to prevent them from being dropped
    struct ClosureHandlers {
        _on_message: Closure<dyn FnMut(MessageEvent)>,
        _on_close: Closure<dyn FnMut(CloseEvent)>,
        _on_error: Closure<dyn FnMut(ErrorEvent)>,
        _on_open: Closure<dyn FnMut(web_sys::Event)>,
    }

    // Simple message type for the WebSocket
    pub enum WsMessage {
        Binary(Vec<u8>),
        Close,
        Error(String),
    }

    impl WasmWebSocket {
        pub async fn connect(url: &str) -> Result<Self> {
            let connection_id = CONNECTION_COUNTER.fetch_add(1, Ordering::Relaxed);
            log(&format!(
                "DEBUG: Creating WebSocket connection #{} to {}",
                connection_id, url
            ));

            // Create the WebSocket - handle JS exceptions properly
            let ws = WebSocket::new(url).map_err(|_| {
                VfsError::WebSocketError("Failed to create WebSocket connection".to_string())
            })?;

            // Set binary type
            ws.set_binary_type(BinaryType::Arraybuffer);

            // Create channels for messages
            let (sender, receiver) = mpsc::unbounded();
            // Use Rc<RefCell<>> for safe sharing instead of leaking
            let sender_id = SENDER_COUNTER.fetch_add(1, Ordering::Relaxed);
            let sender_rc = Rc::new(RefCell::new(sender));
            log(&format!(
                "DEBUG: Connection #{} created sender #{} with Rc<RefCell<>>",
                connection_id, sender_id
            ));

            // Create a oneshot channel to signal connection status
            let (conn_tx, conn_rx) = oneshot::channel::<Result<()>>();
            let conn_tx = std::rc::Rc::new(std::cell::RefCell::new(Some(conn_tx)));

            // Set up open handler to signal successful connection
            let conn_tx_open = conn_tx.clone();
            let on_open = Closure::wrap(Box::new(move |_: web_sys::Event| {
                log(&format!(
                    "DEBUG: Connection #{} - onopen closure invoked",
                    connection_id
                ));
                if let Some(tx) = conn_tx_open.borrow_mut().take() {
                    let _ = tx.send(Ok(()));
                }
            }) as Box<dyn FnMut(web_sys::Event)>);
            ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
            log(&format!(
                "DEBUG: Connection #{} - onopen closure created",
                connection_id
            ));

            // Set up error handler to signal connection failure
            let conn_tx_error = conn_tx.clone();
            let on_error = Closure::wrap(Box::new(move |e: ErrorEvent| {
                // Safely get error message, use a default if the message is empty or undefined
                let error_msg = if e.message().is_empty() {
                    "WebSocket connection error".to_string()
                } else {
                    e.message()
                };
                let error =
                    VfsError::WebSocketError(format!("WebSocket connection failed: {}", error_msg));
                if let Some(tx) = conn_tx_error.borrow_mut().take() {
                    let _ = tx.send(Err(error));
                }
            }) as Box<dyn FnMut(ErrorEvent)>);
            ws.set_onerror(Some(on_error.as_ref().unchecked_ref()));

            // Set up message handler
            let sender_for_msg = sender_rc.clone();
            let conn_id_for_msg = connection_id;
            let on_message = Closure::wrap(Box::new(move |e: MessageEvent| {
                log(&format!(
                    "DEBUG: Connection #{} - onmessage closure invoked",
                    conn_id_for_msg
                ));
                if let Ok(array_buffer) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
                    let array = Uint8Array::new(&array_buffer);
                    let bytes = array.to_vec();
                    log(&format!(
                        "DEBUG: Connection #{} - sending {} bytes to channel",
                        conn_id_for_msg,
                        bytes.len()
                    ));
                    if let Ok(sender) = sender_for_msg.try_borrow() {
                        let _ = sender.unbounded_send(WsMessage::Binary(bytes));
                    } else {
                        log(&format!(
                            "DEBUG: Connection #{} - sender already borrowed!",
                            conn_id_for_msg
                        ));
                    }
                } else {
                    log(&format!(
                        "DEBUG: Connection #{} - received non-binary message",
                        conn_id_for_msg
                    ));
                }
            }) as Box<dyn FnMut(MessageEvent)>);
            ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
            log(&format!(
                "DEBUG: Connection #{} - onmessage closure created",
                connection_id
            ));

            // Set up close handler
            let sender_for_close = sender_rc.clone();
            let conn_id_for_close = connection_id;
            let on_close = Closure::wrap(Box::new(move |_e: CloseEvent| {
                log(&format!(
                    "DEBUG: Connection #{} - onclose closure invoked",
                    conn_id_for_close
                ));
                if let Ok(sender) = sender_for_close.try_borrow() {
                    let _ = sender.unbounded_send(WsMessage::Close);
                } else {
                    log(&format!(
                        "DEBUG: Connection #{} - close sender already borrowed!",
                        conn_id_for_close
                    ));
                }
            }) as Box<dyn FnMut(CloseEvent)>);
            ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
            log(&format!(
                "DEBUG: Connection #{} - onclose closure created",
                connection_id
            ));

            // Wait for connection to complete (success or error)
            let connection_result = conn_rx.await.map_err(|_| {
                VfsError::WebSocketError("Connection attempt was cancelled".to_string())
            })?;

            // Return error if connection failed
            connection_result?;

            let websocket = Self {
                ws,
                _closures: ClosureHandlers {
                    _on_message: on_message,
                    _on_close: on_close,
                    _on_error: on_error,
                    _on_open: on_open,
                },
                receiver,
                connection_id,
            };

            log(&format!(
                "DEBUG: Connection #{} - WebSocket created successfully",
                connection_id
            ));
            log(&format!(
                "DEBUG: Connection #{} - Sender is kept alive by closures",
                connection_id
            ));
            Ok(websocket)
        }

        pub fn send_binary(&self, data: Vec<u8>) -> Result<()> {
            let array = Uint8Array::from(&data[..]);
            self.ws
                .send_with_array_buffer(&array.buffer())
                .map_err(|e| {
                    VfsError::WebSocketError(format!("Failed to send message: {:?}", e))
                })?;
            Ok(())
        }

        pub fn close(&self) {
            let _ = self.ws.close();
        }
    }

    // WASM is single-threaded, so we can safely implement Send
    unsafe impl Send for WasmWebSocket {}

    // Implement Stream for WasmWebSocket to make it work with samod
    impl Stream for WasmWebSocket {
        type Item = std::result::Result<Vec<u8>, VfsError>;

        fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            match Pin::new(&mut self.receiver).poll_next(cx) {
                Poll::Ready(Some(WsMessage::Binary(data))) => {
                    log(&format!(
                        "DEBUG: Connection #{} - Stream yielding {} bytes",
                        self.connection_id,
                        data.len()
                    ));
                    Poll::Ready(Some(Ok(data)))
                }
                Poll::Ready(Some(WsMessage::Close)) => {
                    log(&format!(
                        "DEBUG: Connection #{} - Stream closed",
                        self.connection_id
                    ));
                    Poll::Ready(None)
                }
                Poll::Ready(Some(WsMessage::Error(e))) => {
                    log(&format!(
                        "DEBUG: Connection #{} - Stream error: {}",
                        self.connection_id, e
                    ));
                    Poll::Ready(Some(Err(VfsError::WebSocketError(e))))
                }
                Poll::Ready(None) => {
                    log(&format!(
                        "DEBUG: Connection #{} - Stream ended",
                        self.connection_id
                    ));
                    Poll::Ready(None)
                }
                Poll::Pending => Poll::Pending,
            }
        }
    }

    // Implement Sink for WasmWebSocket to make it work with samod
    impl Sink<Vec<u8>> for WasmWebSocket {
        type Error = VfsError;

        fn poll_ready(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), Self::Error>> {
            // WebSocket is always ready to accept messages
            Poll::Ready(Ok(()))
        }

        fn start_send(self: Pin<&mut Self>, item: Vec<u8>) -> std::result::Result<(), Self::Error> {
            log(&format!(
                "DEBUG: Connection #{} - Sink sending {} bytes of sync data via WebSocket",
                self.connection_id,
                item.len()
            ));
            let result = self.send_binary(item);
            match &result {
                Ok(_) => log(&format!(
                    "DEBUG: Connection #{} - Sink successfully sent data",
                    self.connection_id
                )),
                Err(e) => log(&format!(
                    "DEBUG: Connection #{} - Sink failed to send data: {:?}",
                    self.connection_id, e
                )),
            }
            result
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), Self::Error>> {
            // WebSocket sends messages immediately, no buffering to flush
            Poll::Ready(Ok(()))
        }

        fn poll_close(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), Self::Error>> {
            log(&format!(
                "DEBUG: Connection #{} - Sink closing",
                self.connection_id
            ));
            let _ = self.close();
            Poll::Ready(Ok(()))
        }
    }

    // Helper to connect samod to a WebSocket in WASM
    pub async fn connect_websocket_to_samod(samod: Arc<Repo>, url: &str) -> Result<()> {
        log(&format!("DEBUG: Starting samod connection to {}", url));
        let ws = WasmWebSocket::connect(url).await?;
        let connection_id = ws.connection_id;
        log(&format!(
            "DEBUG: Got WebSocket connection #{} for samod",
            connection_id
        ));

        // Create channels to keep the connection alive
        let (keep_alive_tx, mut keep_alive_rx) = mpsc::unbounded::<()>();

        // Spawn the main connection task
        wasm_bindgen_futures::spawn_local(async move {
            log(&format!(
                "WASM: Connection #{} - Starting samod connection",
                connection_id
            ));

            // Split the WebSocket into stream and sink
            let (sink, stream) = ws.split();
            log(&format!(
                "WASM: Connection #{} - WebSocket split, calling samod.connect()",
                connection_id
            ));

            // Run the connection - this future will complete when the connection ends
            let result = samod.connect(stream, sink, ConnDirection::Outgoing).await;
            log(&format!(
                "WASM: Connection #{} - Finished with result: {:?}",
                connection_id, result
            ));

            // Drop the sender to signal completion
            drop(keep_alive_tx);
        });

        // Spawn a task to keep the connection alive until it completes
        wasm_bindgen_futures::spawn_local(async move {
            log(&format!(
                "DEBUG: Connection #{} - Keep-alive task started",
                connection_id
            ));
            // This will wait until the sender is dropped (connection completes)
            while keep_alive_rx.next().await.is_some() {}
            log(&format!(
                "DEBUG: Connection #{} - Keep-alive task ending, connection closed",
                connection_id
            ));
        });

        log(&format!(
            "DEBUG: Connection #{} - Setup complete, connection is running",
            connection_id
        ));
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub mod native_impl {
    use super::*;
    use samod::{ConnDirection, ConnFinishedReason, Repo};
    use tokio_tungstenite::connect_async;

    pub async fn connect_websocket_to_samod(
        samod: Arc<Repo>,
        url: &str,
    ) -> Result<ConnFinishedReason> {
        let (ws_stream, _) = connect_async(url)
            .await
            .map_err(|e| VfsError::WebSocketError(format!("Failed to connect to {url}: {e}")))?;

        // Use samod's built-in WebSocket support
        Ok(samod
            .connect_tungstenite(ws_stream, ConnDirection::Outgoing)
            .await)
    }
}
