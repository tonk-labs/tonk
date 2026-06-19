//! `<tonk-credential>` — a local-keypair source for declarative views.
//!
//! On connect it generates a fresh per-instance Ed25519 keypair *in the
//! browser* and distributes these values to descendants that ask for them
//! via `bind:` attributes:
//!
//! - `did`  — the public `did:key:…` identifier of the keypair.
//! - `seed` — base58 of the 32-byte private seed.
//! - `base` — the invite-URL base (`{origin}/join`), so a template can
//!   assemble an absolute link without reading `window` itself.
//!
//! A descendant opts in by declaring `bind:<name>=<target-attr>`: the
//! element writes the named value into that target attribute on the
//! descendant. So
//!
//! ```html
//! <tonk-credential>
//!   <input        name=audience bind:did=value>
//!   <tonk-display bind:did=entity bind:seed=data-seed model=invitation>
//!     <wa-input readonly value="?access={access}#{dom.host/data-seed}">
//!     </wa-input>
//!   </tonk-display>
//! </tonk-credential>
//! ```
//!
//! threads the public DID into the form input (the command reads it as
//! the invite audience) and into the nested `<tonk-display>`'s `entity`
//! (so it resolves the `invitation` fact the worker asserts), and the
//! private seed into the display's `data-seed` (read back in-template as
//! `{dom.host/data-seed}` to compose the final URL). The seed never
//! leaves the DOM: only the public DID crosses to the worker.
//!
//! The element is deliberately ignorant of `<tonk-display>`, `entity`,
//! invites, and URLs — it only generates a keypair and fills whatever
//! targets its descendants declare. The descendant markup, not the
//! element, owns the wiring.

use custom_elements::CustomElement;
use dialog_credentials::{Ed25519Signer, KeyExport};
use dialog_varsig::Principal as _;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, HtmlElement, window};

/// The `bind:` attribute prefix a descendant uses to request a value.
const BIND_PREFIX: &str = "bind:";

/// Per-element state. The element holds nothing across renders today —
/// the keypair is generated on connect and its values are pushed into
/// the DOM — so the struct is empty.
#[derive(Default)]
pub(crate) struct TonkCredential;

impl CustomElement for TonkCredential {
    fn shadow() -> bool {
        // Light DOM: the element must see its descendants to read their
        // `bind:` declarations and write their target attributes.
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let host = this.clone();
        spawn_local(async move {
            match generate().await {
                Some((did, seed)) => distribute(&host, &did, &seed, &join_base()),
                None => tonk_common::log!("tonk-credential: keypair generation failed"),
            }
        });
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {}
}

/// Generate a fresh keypair and return its `(did, seed_base58)`.
///
/// The seed must be embeddable in the invite URL, so the key has to be
/// *extractable*. Wasm's default `Ed25519Signer::generate` produces a
/// non-extractable WebCrypto key (the secure default), so we opt into
/// extractable generation via [`ExtractableKey`] — the same path the
/// worker's `generate_ephemeral` uses.
async fn generate() -> Option<(String, String)> {
    use dialog_credentials::key::ExtractableKey;
    let signer = <Ed25519Signer as ExtractableKey>::generate().await.ok()?;
    let did = signer.did().as_str().to_owned();
    let seed = match signer.export().await.ok()? {
        KeyExport::Extractable(bytes) => bs58::encode(bytes).into_string(),
        KeyExport::NonExtractable { .. } => return None,
    };
    Some((did, seed))
}

/// The invite URL base — the recipient's `/join` page on this origin.
/// Provided as the `base` bind value so a template can assemble an
/// absolute invite URL (`{base}?access=…#{seed}`) without reading
/// `window` itself. Empty when there is no window/origin.
fn join_base() -> String {
    window()
        .and_then(|w| w.location().origin().ok())
        .map(|origin| format!("{origin}/join"))
        .unwrap_or_default()
}

/// Walk the element's descendants and, for each `bind:<name>=<target>`
/// attribute, write the matching value into the named target attribute.
fn distribute(host: &HtmlElement, did: &str, seed: &str, base: &str) {
    for element in descendants(host) {
        for (name, target) in bindings(&element) {
            let value = match name.as_str() {
                "did" => Some(did),
                "seed" => Some(seed),
                "base" => Some(base),
                _ => None,
            };
            if let Some(value) = value {
                let _ = element.set_attribute(&target, value);
            }
        }
    }
}

/// Every descendant element of `host`, in document order.
fn descendants(host: &HtmlElement) -> Vec<Element> {
    let mut out = Vec::new();
    if let Ok(nodes) = host.query_selector_all("*") {
        for i in 0..nodes.length() {
            if let Some(element) = nodes.item(i).and_then(|n| n.dyn_into::<Element>().ok()) {
                out.push(element);
            }
        }
    }
    out
}

/// Read an element's `bind:<name>=<target>` attributes as
/// `(name, target)` pairs. A `querySelector` for `[bind:…]` would need
/// the colon escaped, so we iterate the attribute list instead.
fn bindings(element: &Element) -> Vec<(String, String)> {
    let attrs = element.attributes();
    let mut out = Vec::new();
    for i in 0..attrs.length() {
        let Some(attr) = attrs.item(i) else { continue };
        let name = attr.name();
        if let Some(value_name) = name.strip_prefix(BIND_PREFIX) {
            // `bind:did=entity` → value `did` into target attribute `entity`.
            out.push((value_name.to_owned(), attr.value()));
        }
    }
    out
}

/// Register `<tonk-credential>`. Idempotent.
pub(crate) fn register() {
    if already_registered() {
        return;
    }
    TonkCredential::define("tonk-credential");
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-credential").is_undefined()
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    /// A connected `<tonk-credential>` fills a child `bind:did=value`
    /// input with a `did:key:` string and a `bind:seed=data-seed`
    /// element with a base58 seed. The keypair is async, so poll until
    /// the values land.
    #[dialog_common::test]
    async fn it_binds_the_did_and_seed_to_declared_targets() {
        register();
        let document = window().unwrap().document().unwrap();
        let body = document.body().unwrap();

        let credential = document.create_element("tonk-credential").unwrap();
        let input = document.create_element("input").unwrap();
        input.set_attribute("bind:did", "value").unwrap();
        credential.append_child(&input).unwrap();
        let sink = document.create_element("div").unwrap();
        sink.set_attribute("bind:seed", "data-seed").unwrap();
        credential.append_child(&sink).unwrap();
        body.append_child(&credential).unwrap();

        // Poll for the async keypair to land in both targets.
        let mut did = String::new();
        let mut seed = String::new();
        for _ in 0..100 {
            did = input
                .dyn_ref::<web_sys::HtmlInputElement>()
                .map(|i| i.value())
                .unwrap_or_default();
            seed = sink.get_attribute("data-seed").unwrap_or_default();
            if !did.is_empty() && !seed.is_empty() {
                break;
            }
            gloo_timer_sleep().await;
        }

        assert!(
            did.starts_with("did:key:"),
            "bind:did should fill the input value with a did:key, got {did:?}",
        );
        assert!(
            !seed.is_empty(),
            "bind:seed should fill data-seed with a base58 seed",
        );

        credential.remove();
    }

    /// Bare descendants with no `bind:` attribute are left untouched.
    #[dialog_common::test]
    async fn it_ignores_descendants_without_a_bind_attribute() {
        register();
        let document = window().unwrap().document().unwrap();
        let body = document.body().unwrap();

        let credential = document.create_element("tonk-credential").unwrap();
        let plain = document.create_element("span").unwrap();
        credential.append_child(&plain).unwrap();
        body.append_child(&credential).unwrap();

        gloo_timer_sleep().await;

        assert!(
            !plain.has_attribute("entity") && !plain.has_attribute("value"),
            "a descendant with no bind: attribute must not be modified",
        );

        credential.remove();
    }

    /// Yield to the microtask/timer queue so the async keypair future
    /// can make progress between polls.
    async fn gloo_timer_sleep() {
        let promise = js_sys::Promise::new(&mut |resolve: js_sys::Function, _reject| {
            let win = window().unwrap();
            win.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 10)
                .unwrap();
        });
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }
}
