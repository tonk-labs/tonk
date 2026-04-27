//! JSON-RPC 2.0 framing types specialized for LSP.
//!
//! LSP rides on JSON-RPC 2.0 with a few additional rules: requests
//! always carry an `id`, notifications never do, and responses echo
//! the request `id`. We model the wire format with explicit `Request`
//! / `Notification` / `Response` variants rather than a single
//! `Message` struct so the dispatcher can pattern-match on intent
//! without sniffing fields.
//!
//! The full `lsp-types` crate covers the *content* of each method's
//! `params` and `result`, but it deliberately doesn't model the
//! envelope. We supply that envelope here.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC `id` field. The spec allows numbers, strings, or null;
/// we pass the original through untouched so responses match exactly.
pub type RequestId = Value;

/// An incoming message — either a request expecting a response, a
/// notification with no expected reply, or a response to a server-
/// initiated request (rare for an LSP server, included for
/// completeness).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Incoming {
    /// `id` and `method` both present → request.
    Request(Request),
    /// `method` present, no `id` → notification.
    Notification(Notification),
    /// `id` and `result`/`error` → response. Server-initiated requests
    /// are not used in phase 0, but the variant exists so a stray
    /// response doesn't deserialize as a malformed request.
    ///
    /// The `id` field is the discriminator that prevents
    /// `serde(untagged)` from matching arbitrary objects against
    /// this variant — without it, the unit-shape arm would catch
    /// every JSON object and break classification.
    Response {
        #[serde(rename = "id")]
        _id: RequestId,
    },
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: RequestId,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: RequestId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
}

impl Response {
    pub fn success(id: RequestId, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: RequestId, error: ResponseError) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// A server-initiated notification. We use this for
/// `textDocument/publishDiagnostics` and any future push
/// notifications (`window/showMessage`, etc.).
#[derive(Debug, Serialize)]
pub struct OutboundNotification {
    pub jsonrpc: &'static str,
    pub method: &'static str,
    pub params: Value,
}

impl OutboundNotification {
    pub fn new(method: &'static str, params: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            method,
            params,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ResponseError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl ResponseError {
    /// JSON-RPC 2.0 reserved error codes we currently emit.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;

    pub fn invalid_params(msg: impl Into<String>) -> Self {
        Self {
            code: Self::INVALID_PARAMS,
            message: msg.into(),
            data: None,
        }
    }
    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: Self::METHOD_NOT_FOUND,
            message: format!("method not found: {method}"),
            data: None,
        }
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        Self {
            code: Self::INTERNAL_ERROR,
            message: msg.into(),
            data: None,
        }
    }
}
