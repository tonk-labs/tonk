//! Forwarding for page-only effects.
//!
//! Some effects can only happen on the top page: moving `location`,
//! setting `document.title`, opening a tab. The chrome that wants them
//! runs in a sealed guest, so it posts a message over the portal bridge
//! — and the bridge dispatcher runs in the guest's PARENT, which is not
//! necessarily the page.
//!
//! Space content is a guest inside the profile chrome, which is itself a
//! guest. A message from space content is dispatched one hop up, in an
//! opaque-origin `about:srcdoc` document, where performing the effect
//! either throws or corrupts the wrong frame. See the design spec.
//!
//! So every page effect asks this first: am I myself a guest? If so the
//! effect is re-posted to the parent, which asks the same question. The
//! recursion terminates at the page, in O(frames) hops. It is the shape
//! `bridge::context_origin` already uses for "the real value lives N
//! frames up".
//!
//! `navigate.rs:9-14` called for exactly this registry when a second page
//! effect appeared. `title` arrived as a parallel special case instead;
//! this is the generalization, and `open` is the third member.
//!
//! NOTE: unrelated to `depth.rs`, which counts DOM consumer nesting
//! inside a single document.

use js_sys::{Function, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use web_sys::window;

/// Post a page effect to the parent when this document is a portal guest.
///
/// Returns `true` when the caller must stop — either the effect was
/// forwarded, or this is a guest whose bridge could not carry it (in which
/// case dropping it is still correct: a guest must never perform a page
/// effect on its own document). Returns `false` only for a real page, which
/// must perform the effect itself.
///
/// The discriminator is `window.tonk`. Two places install one, so the
/// invariant is not "only the bootstrap assigns it" — it is that
/// **a realm that loads `bridge.js` never runs the element runtime**:
///
/// - `BOOTSTRAP_JS` (`tonk-portal/src/bridge.rs`) installs the bridge this
///   module posts to. It is parent-pushed into a guest's `srcdoc`, and
///   `shared.rs` always sets `srcdoc`, never `src` — so every realm the
///   element runtime enters got there this way.
/// - `tonk-worker/assets/bridge.js` also does `globalThis.tonk = bridge`
///   (`worker.rs` documents it). It is loaded only by `wrap_html_body`
///   (`tonk-worker/src/router/host.rs`), which serves agent-authored HTML
///   into an SW-routed `src` iframe — a realm with no element runtime in it,
///   and so no caller of `forward`.
///
/// The two never meet, so wherever `forward` runs, a present `tonk` is the
/// portal bridge and its presence means precisely "I am a portal guest with a
/// bridge to my parent". Deliberately NOT `window === window.top`, which
/// encodes "I am the outermost frame" — a different claim that would break if
/// the Tonk page were ever itself embedded.
///
/// VIOLATE THAT AND EVERY PAGE EFFECT DIES AT ONCE, SILENTLY. Adding the
/// element runtime to `wrap_html_body`, or `bridge.js` to a runtime guest,
/// makes `forward` find a `tonk` with no `navigate`/`reload`/`setTitle`/`open`: it
/// takes the guest branch, drops the effect, and returns `true`. Installing a
/// `window.tonk` of either kind on the TOP page does the same there. That is
/// the assumption to check first if navigation, reloads, titles, and link
/// opening all stop working together.
pub(crate) fn forward(method: &str, arg: &str) -> bool {
    let Some(win) = window() else {
        return false;
    };
    let Ok(tonk) = Reflect::get(&win, &JsValue::from_str("tonk")) else {
        return false;
    };
    if tonk.is_undefined() || tonk.is_null() {
        return false;
    }
    // A guest from here on: whatever happens, do not perform locally.
    if let Ok(method) = Reflect::get(&tonk, &JsValue::from_str(method))
        && let Ok(method) = method.dyn_into::<Function>()
    {
        let _ = method.call1(&tonk, &JsValue::from_str(arg));
    }
    true
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use js_sys::{Array, Function, Object, Reflect};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::JsValue;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    /// Install a stub `window.tonk` whose `method` records its argument into
    /// a JS array, and hand back that array. Every test that calls this MUST
    /// call `clear_tonk()` before returning: `window` is shared across every
    /// test in this wasm module, and a leaked `window.tonk` would make each
    /// page effect silently forward into the void for every test that runs
    /// afterwards.
    fn install_tonk(method: &str) -> Array {
        let calls = Array::new();
        let recorder = {
            let calls = calls.clone();
            Closure::wrap(Box::new(move |value: JsValue| {
                calls.push(&value);
            }) as Box<dyn FnMut(JsValue)>)
        };
        let tonk = Object::new();
        let _ = Reflect::set(
            &tonk,
            &JsValue::from_str(method),
            recorder.as_ref().unchecked_ref::<Function>(),
        );
        // The stub must outlive this fn; the test clears `window.tonk` instead.
        recorder.forget();
        let win = web_sys::window().expect("a window in the test harness");
        let _ = Reflect::set(&win, &JsValue::from_str("tonk"), &tonk);
        calls
    }

    fn clear_tonk() {
        let win = web_sys::window().expect("a window in the test harness");
        let _ = Reflect::delete_property(win.unchecked_ref::<Object>(), &JsValue::from_str("tonk"));
    }

    /// A document with a `window.tonk` is a portal guest: the effect is posted
    /// to the parent and the caller is told to stop.
    #[dialog_common::test]
    async fn it_forwards_when_this_document_is_a_guest() {
        let calls = install_tonk("navigate");

        let forwarded = forward("navigate", "/space/abc");
        let call_count = calls.length();
        let first_arg = calls.get(0).as_string();

        // Restore before asserting so a failure doesn't leak `window.tonk`
        // into a later test running in the same page.
        clear_tonk();

        assert!(forwarded, "a guest should forward the effect");
        assert_eq!(call_count, 1, "the parent should have been called once");
        assert_eq!(
            first_arg,
            Some("/space/abc".to_owned()),
            "the href should reach the parent verbatim"
        );
    }

    /// The top page has no `window.tonk` — it must perform the effect itself.
    #[dialog_common::test]
    async fn it_does_not_forward_when_this_document_is_the_page() {
        clear_tonk();

        assert!(
            !forward("navigate", "/space/abc"),
            "the top page should perform the effect, not forward it"
        );
    }

    /// A guest whose bridge lacks the method cannot forward. Reporting `false`
    /// would make the caller perform a page effect inside an iframe; reporting
    /// `true` drops it. Dropping is correct — performing is the bug this whole
    /// module exists to prevent.
    #[dialog_common::test]
    async fn it_drops_rather_than_performs_when_the_bridge_lacks_the_method() {
        let _calls = install_tonk("navigate");

        let dropped = forward("setTitle", "Notes — Tonk");
        clear_tonk();

        assert!(
            dropped,
            "a guest missing the method should still not perform locally"
        );
    }
}
