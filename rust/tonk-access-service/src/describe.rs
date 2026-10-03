//! An invocation described in response headers, for whoever reads a
//! network log: every request to `/ucan/` otherwise looks the same.
//!
//! The command and the subject also ride in the request's query, which
//! a log shows by name. The arguments do not, since a URL that differs
//! per request would cost a preflight per request; they come back here
//! instead, where a log shows them as columns and a HAR keeps them.

use base58::ToBase58;
use dialog_ucan_core::InvocationChain;
use dialog_ucan_core::promise::Promised;
use dialog_varsig::AnySignature;

/// The header naming the invocation's command.
pub const COMMAND: &str = "UCAN-Command";
/// The header naming the invocation's subject.
pub const SUBJECT: &str = "UCAN-Subject";
/// The header carrying the invocation's arguments, `name=value` pairs
/// separated by `; `, bytes in base58.
pub const ARGUMENTS: &str = "UCAN-Arguments";

/// The headers describing `chain`, in the order they read best.
pub fn describe(chain: &InvocationChain<AnySignature>) -> [(&'static str, String); 3] {
    let arguments = chain
        .arguments()
        .iter()
        .map(|(name, value)| format!("{name}={}", render(value)))
        .collect::<Vec<_>>()
        .join("; ");
    [
        (COMMAND, chain.command().to_string()),
        (SUBJECT, chain.subject().to_string()),
        (ARGUMENTS, arguments),
    ]
}

/// A value as a log reads it: text and numbers as they are, bytes in
/// base58, the rest by shape.
fn render(value: &Promised) -> String {
    match value {
        Promised::Null => "null".to_string(),
        Promised::Bool(value) => value.to_string(),
        Promised::Integer(value) => value.to_string(),
        Promised::Float(value) => value.to_string(),
        Promised::String(value) => value.clone(),
        Promised::Bytes(bytes) => bytes.to_base58(),
        Promised::Link(cid) => cid.to_string(),
        Promised::WaitOk(cid) | Promised::WaitErr(cid) | Promised::WaitAny(cid) => {
            format!("await {cid}")
        }
        Promised::List(items) => format!(
            "[{}]",
            items.iter().map(render).collect::<Vec<_>>().join(", ")
        ),
        Promised::Map(fields) => format!(
            "{{{}}}",
            fields
                .iter()
                .map(|(name, value)| format!("{name}={}", render(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}
