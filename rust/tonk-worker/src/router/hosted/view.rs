//! Views for `/mcp` hosts (MCP Apps): extra tools and the resources they
//! render. A seam for now: [`super::mcp`] offers whatever this module
//! returns, and it returns nothing.

use serde_json::Value;

use super::mcp::Verified;

/// The key view tickets are bound under, derived from the host's secret
/// under its own context, apart from the session id MAC.
#[derive(Clone)]
pub struct TicketKey(#[allow(dead_code)] [u8; 32]);

impl TicketKey {
    /// Derive the key from `secret`, the host's MCP secret.
    pub fn derive(secret: &str) -> Self {
        Self(blake3::derive_key(
            "tonk hosted view ticket v1",
            secret.as_bytes(),
        ))
    }
}

/// Tools appended to `tools/list`.
pub fn tools() -> Vec<Value> {
    Vec::new()
}

/// Resources served by `resources/list`.
pub fn resources() -> Vec<Value> {
    Vec::new()
}

/// The `resources/read` result for `uri`, or `None` when there is no such
/// resource.
pub fn read_resource(_uri: &str, _origin: &str) -> Option<Value> {
    None
}

/// The result of the view tool `name`, or `None` when it is not one.
pub async fn call(
    _name: &str,
    _arguments: &Value,
    _verified: &Verified,
    _bearer: &str,
    _mcp_session: &str,
    _origin: &str,
    _tickets: &TicketKey,
) -> Option<Value> {
    None
}
