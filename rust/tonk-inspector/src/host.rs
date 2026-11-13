//! What the notebook elements ask of the host: the branch an element's own
//! `with` names, and an evaluation against it.

use tonk_host::consumer;
use tonk_host::location::Location;
use wasm_bindgen::JsValue;
use web_sys::{Element, HtmlElement};

use crate::response::EvaluateResponse;

/// The `(repo, branch)` an element's OWN `with="branch@repo"` names
/// (forwarded onto it by the mounting `<tonk-display>`; routing is never
/// inferred from DOM ancestors).
///
/// The pair scopes the cells' LSP buffer URIs; the evaluate itself routes
/// through the host consumer, which resolves the route from the element.
///
/// A profile context keeps its `profile:<name>` repo token rather than
/// flattening to a bare `profile`: the language server parses this segment
/// back out of the buffer URI to decide which namespace to open, and the
/// profile is not reachable as a repository *named* `profile`.
pub(crate) struct Context {
    pub(crate) repo: String,
    pub(crate) branch: String,
}

/// The [`Context`] `element`'s own `with` names, or `None` when it is
/// absent or still an unstamped template placeholder.
pub(crate) fn resolve_context(element: &HtmlElement) -> Option<Context> {
    let location: Location = element
        .get_attribute("with")
        .filter(|v| !v.is_empty() && !v.contains('{'))
        .and_then(|v| v.parse().ok())?;
    let repo = match location.space() {
        Some(name) => name.to_owned(),
        // `Repo::Profile`'s `Display` is exactly the `profile:<name>`
        // token; a location with no branch renders bare, so build the
        // segment from the repo half alone.
        None => location.repo.to_string(),
    };
    Some(Context {
        repo,
        branch: location.effective_branch().to_owned(),
    })
}

/// Evaluate `document` against the branch, through the host consumer.
///
/// Dispatches a `tonk-evaluate` event on `consumer`; the host (in the
/// sealed guest, the proxy relaying over the bridge) owns the request and
/// returns the parsed response, which round-trips through `serde_json`
/// into the local mirror.
pub(crate) async fn evaluate(
    consumer: &Element,
    document: &str,
    transact: bool,
) -> Result<EvaluateResponse, String> {
    let value = consumer::evaluate(consumer, document, transact)
        .await
        .map_err(|detail| detail.message)?;
    // `value` is a `JsValue` from `JSON.parse`; stringify + serde so the field
    // maps decode with the same semantics as the worker's own `Deserialize`.
    let text = js_sys::JSON::stringify(&value)
        .ok()
        .and_then(|s| s.as_string())
        .ok_or_else(|| "evaluate response was not serializable JSON".to_owned())?;
    serde_json::from_str(&text).map_err(|e| format!("evaluate response decode: {e}"))
}

/// The string at `key` on `value`, if it is one.
pub(crate) fn reflect_string(value: &JsValue, key: &str) -> Option<String> {
    js_sys::Reflect::get(value, &JsValue::from_str(key))
        .ok()
        .and_then(|v| v.as_string())
}
