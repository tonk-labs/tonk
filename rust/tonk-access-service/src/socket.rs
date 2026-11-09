//! What the worker and the local server share about a space's socket:
//! which invocations change what its watches see.

use dialog_ucan_core::InvocationChain;

/// The cell `chain`'s invocation writes, `(space, cell)`, when it is a
/// cell's publish or retract.
pub fn cell_written<S: dialog_varsig::Signature>(
    chain: &InvocationChain<S>,
) -> Option<(String, String)> {
    let command: Vec<&str> = chain.command().0.iter().map(String::as_str).collect();
    if !matches!(
        command.as_slice(),
        ["use", "put", "memory", "cell"] | ["use", "delete", "memory", "cell"]
    ) {
        return None;
    }
    let arguments = chain.arguments();
    let named = |key: &str| match arguments.get(key) {
        Some(dialog_ucan_core::promise::Promised::String(value)) => Some(value.clone()),
        _ => None,
    };
    Some((named("space")?, named("cell")?))
}
