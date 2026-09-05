//! `<tonk-invite-link>` — turn a pasted invite URL into a local `/join`.
//!
//! A long invite link carries everything a join needs in its query and
//! fragment — the delegation chain, the space's sync remote, the
//! revocation relay — so the origin it was minted on is only a doorway.
//! Pasting a link minted elsewhere (say, production) into this deployment
//! joins the space here while its blocks keep coming from the remote the
//! invite names — which is exactly what local debugging against real
//! data wants.
//!
//! Minted links are usually SHORT, though: `{origin}/@/{hash}#{seed}`,
//! where the chain and remote live behind that origin's shortcut service
//! and only the seed rides the fragment. Those must be expanded on the
//! origin that minted them — a local shortcut service has never seen the
//! hash. So this element resolves a short link first (`HEAD /@/{hash}`
//! answers a relative `Location`, and both that route and the `/join` it
//! redirects to allow this origin), then rewrites the expanded query
//! onto this deployment's `/join`. The seed never leaves the browser: it
//! is a fragment, so it is never sent with the request, and this code
//! carries it across from the pasted link itself.
//!
//! Static notation can't read an input, fetch, or navigate, so this dumb
//! element bridges the gap the same way [`super::default_remote`] does
//! for the origin. All join policy stays in the worker's claim pipeline.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{Event, HtmlElement, window};

/// A retained submit-listener closure, kept alive for the element's
/// lifetime so the listener stays valid.
type SubmitClosure = Rc<RefCell<Option<Closure<dyn FnMut(Event)>>>>;

/// The form-control `name` read when the element sets no `field`.
const DEFAULT_FIELD: &str = "url";

/// The query parameter naming an invite's delegation chain. Its presence
/// is what makes a pasted link self-contained.
const ACCESS_PARAM: &str = "access";

/// Attribute the element stamps with the outcome of the last submit —
/// `invalid` when the pasted text isn't an invite URL — so the template
/// can style an error state without any script of its own.
const STATE_ATTR: &str = "data-state";

/// Why pasted text could not become a local join target. Kept on the
/// element as `data-refusal` so the light-DOM wall can explain and animate
/// the exact refusal without parsing URLs a second way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InviteLinkRefusal {
    Empty,
    Malformed,
    Unresolvable,
}

impl InviteLinkRefusal {
    fn as_attr(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Malformed => "malformed",
            Self::Unresolvable => "unresolvable",
        }
    }
}

#[derive(Default)]
pub(crate) struct TonkInviteLink {
    submit: SubmitClosure,
}

impl CustomElement for TonkInviteLink {
    fn shadow() -> bool {
        // Light DOM: the element must see its `<form>` ancestor.
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let Ok(Some(form)) = this.closest("form") else {
            return;
        };
        let host = this.clone();
        let listener = Closure::wrap(Box::new(move |event: Event| {
            // The submit never reaches the network: this element resolves
            // the pasted text into a redeemable `/join` URL and hands it
            // to the bound `tonk:join` command as a `mount` event, the
            // same shape `<tonk-page>` fires on a real page load. That is
            // why the form binds `onmount` and not `onsubmit`: `Join`
            // reads `dom.event.detail/href`, an event-read path a submit
            // event has no way to satisfy.
            //
            // Always cancelled: even an already-complete link has to be
            // re-delivered as `detail.href`, and resolving a short one is
            // async besides.
            event.prevent_default();
            let host = host.clone();
            wasm_bindgen_futures::spawn_local(async move { submit(&host).await });
        }) as Box<dyn FnMut(Event)>);
        let _ = form.add_event_listener_with_callback("submit", listener.as_ref().unchecked_ref());
        *self.submit.borrow_mut() = Some(listener);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.submit.borrow_mut().take();
    }
}

/// Resolve the pasted text and hand the redeemable URL to the bound
/// command as a `mount` event.
///
/// The invite has to arrive as `detail.href` because that is the single
/// read path `Join` declares (`dom.event.detail/href`), and a concept's
/// `the:` is one slot serving as both the event read path and the stored
/// attribute — there is no second place to say "read it from here, file
/// it under there". So rather than a second command shaped around the
/// form, the form speaks the shape the existing command already reads,
/// and one handler serves both the pasted link and the visited one.
async fn submit(this: &HtmlElement) {
    let field = this
        .get_attribute("field")
        .unwrap_or_else(|| DEFAULT_FIELD.to_string());
    let pasted = read_field(this, &field).unwrap_or_default();
    let _ = this.set_attribute(STATE_ATTR, "resolving");
    match local_join_href(pasted.trim()).await {
        Ok(href) => {
            let _ = this.remove_attribute(STATE_ATTR);
            let _ = this.remove_attribute("data-refusal");
            dispatch_mount(this, &href);
        }
        Err(refusal) => {
            let _ = this.set_attribute(STATE_ATTR, "invalid");
            let _ = this.set_attribute("data-refusal", refusal.as_attr());
        }
    }
}

/// Fire a bubbling `mount` carrying the resolved invite, mirroring the
/// flat URL-shaped detail `<tonk-page>` builds from a real location so
/// both paths present the bound command with the same event.
///
/// Dispatched from the element itself so it bubbles to the `onmount`
/// binding on the enclosing form, and on to the display's delegate.
fn dispatch_mount(this: &HtmlElement, href: &str) {
    let Ok(url) = web_sys::Url::new_with_base(href, &page_origin()) else {
        return;
    };
    let detail = js_sys::Object::new();
    let set = |key: &str, value: &str| {
        let _ = js_sys::Reflect::set(&detail, &JsValue::from_str(key), &JsValue::from_str(value));
    };
    set("href", &url.href());
    set("origin", &url.origin());
    set("pathname", &url.pathname());
    set("search", &url.search());
    set("hash", &url.hash());

    let init = web_sys::CustomEventInit::new();
    init.set_bubbles(true);
    init.set_detail(&detail);
    if let Ok(event) = web_sys::CustomEvent::new_with_event_init_dict("mount", &init) {
        let _ = this.dispatch_event(&event);
    }
}

/// The origin a relative `/join` URL resolves against. Inside the sealed
/// guest `window.location` is the guest's own `about:srcdoc`, so prefer
/// the host-forwarded origin the bridge publishes; the fallback covers
/// the top-page and test cases.
fn page_origin() -> String {
    window()
        .and_then(|win| {
            js_sys::Reflect::get(&win, &JsValue::from_str("tonk"))
                .ok()
                .and_then(|tonk| js_sys::Reflect::get(&tonk, &JsValue::from_str("context")).ok())
                .and_then(|context| {
                    js_sys::Reflect::get(&context, &JsValue::from_str("origin")).ok()
                })
                .and_then(|origin| origin.as_string())
                .filter(|origin| !origin.is_empty())
                .or_else(|| win.location().origin().ok())
        })
        .unwrap_or_default()
}

/// Read the `value` property of the form control named `field` within the
/// closest `<form>`. Property, not attribute: `<wa-input>` is a
/// form-associated custom element and carries its live value there.
fn read_field(this: &HtmlElement, field: &str) -> Option<String> {
    let form = this.closest("form").ok()??;
    let input = form.query_selector(&format!("[name=\"{field}\"]")).ok()??;
    js_sys::Reflect::get(input.as_ref(), &JsValue::from_str("value"))
        .ok()?
        .as_string()
}

/// Rewrite a pasted invite URL into this deployment's `/join` route,
/// expanding a short link on its own origin first.
///
/// The invite's substance is its query (`access`, `remote`, `revocation`)
/// and its fragment (an open invite's seed). The pasted origin and path
/// are deliberately discarded — they only say where the link was minted —
/// EXCEPT for the one thing only that origin can do: expand `/@/{hash}`
/// into the query it stands for.
async fn local_join_href(pasted: &str) -> Result<String, InviteLinkRefusal> {
    if pasted.is_empty() {
        return Err(InviteLinkRefusal::Empty);
    }
    let url = web_sys::Url::new(pasted).map_err(|_| InviteLinkRefusal::Malformed)?;
    // The seed rides the fragment and is never sent to any server; carry
    // it across from the pasted link whichever form the link took.
    let fragment = url.hash();
    let query = if carries_access(&url) {
        url.search()
    } else {
        resolve_invite(&url)
            .await
            .ok_or(InviteLinkRefusal::Unresolvable)?
    };
    Ok(format!("/join{query}{fragment}"))
}

/// Whether the link already carries its delegation chain, and so needs
/// no round-trip to the origin that minted it.
fn carries_access(url: &web_sys::Url) -> bool {
    url.search_params().has(ACCESS_PARAM)
}

/// Resolve a link that carries no chain by following it on the origin
/// that minted it. The shortcut service answers a permanent redirect
/// whose `Location` is the stored path + query, relative and verbatim,
/// so the URL the fetch lands on carries the invite.
///
/// `redirect: "manual"` would give an opaque response no script can read
/// — its status is 0 and its headers are empty — so the redirect is
/// followed normally and the landing URL's query is what the shortcut
/// stored. Both hops must therefore allow this origin: the shortcut
/// route sends `Access-Control-Allow-Origin: *`, and the join route it
/// lands on is granted the same in the deployment's `_headers`.
///
/// `HEAD` rather than `GET`: the landing URL is the whole answer, so
/// there is no reason to download the app shell behind it.
async fn resolve_invite(url: &web_sys::Url) -> Option<String> {
    let response = reqwest::Client::new().head(url.href()).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let landed = web_sys::Url::new(response.url().as_str()).ok()?;
    // Only an answer that actually carries a chain is an invite: a
    // deployment that served its app shell for an unknown link lands
    // here with nothing, and that is a refusal, not an invite.
    carries_access(&landed).then(|| landed.search())
}

/// Register `<tonk-invite-link>`. Idempotent.
pub(crate) fn register() {
    let Some(elements) = window().map(|w| w.custom_elements()) else {
        return;
    };
    if elements.get("tonk-invite-link").is_undefined() {
        TonkInviteLink::define("tonk-invite-link");
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    /// A long link rewrites straight onto the local join route, keeping
    /// query and fragment verbatim; anything carrying no delegation
    /// chain is refused rather than bounced through /join.
    #[dialog_common::test]
    async fn it_rewrites_a_foreign_invite_onto_the_local_join_route() {
        assert_eq!(
            local_join_href("https://staging.tonk.xyz/join?access=abc&remote=https%3A%2F%2Fs#seed")
                .await,
            Ok("/join?access=abc&remote=https%3A%2F%2Fs#seed".to_string()),
        );
        assert_eq!(
            local_join_href("").await,
            Err(InviteLinkRefusal::Empty),
            "empty paste is refused"
        );
        assert_eq!(
            local_join_href("https://staging.tonk.xyz/").await,
            Err(InviteLinkRefusal::Unresolvable),
            "a bare origin carries no invite"
        );
        assert_eq!(
            local_join_href("not a url").await,
            Err(InviteLinkRefusal::Malformed)
        );
    }

    #[dialog_common::test]
    async fn it_returns_a_target_or_a_structured_parse_refusal() {
        let target = format!(
            "{:?}",
            local_join_href("https://staging.tonk.xyz/join?access=abc#seed").await
        );
        assert!(target.contains("/join?access=abc#seed"));

        let refusal = format!("{:?}", local_join_href("definitely not a url").await);
        assert_eq!(
            refusal, "Err(Malformed)",
            "garbage must produce a named refusal instead of a bare absence or panic",
        );
    }

    /// What decides whether a link needs resolving is the chain it
    /// carries, never the shape of its path: the shortener's URL form is
    /// the minting deployment's business, and a link that already holds
    /// `access` is complete no matter what path it sits on.
    #[dialog_common::test]
    async fn it_resolves_links_that_carry_no_delegation_chain() {
        let complete =
            web_sys::Url::new("https://staging.tonk.xyz/join?access=abc&remote=x#seed").unwrap();
        assert!(carries_access(&complete));

        let shortened = web_sys::Url::new("https://staging.tonk.xyz/@/abc123#seed").unwrap();
        assert!(!carries_access(&shortened), "a short link must be resolved");

        // Any other link shape a deployment might mint resolves the same
        // way — nothing here knows what its paths look like.
        let other = web_sys::Url::new("https://tonk.network/i/xyz?utm=mail#seed").unwrap();
        assert!(!carries_access(&other));

        // An invite on an unexpected path is still complete.
        let odd = web_sys::Url::new("https://tonk.network/anything?access=abc").unwrap();
        assert!(carries_access(&odd));
    }

    /// Submitting the form navigates nowhere on an invalid paste and
    /// stamps the error state the template styles.
    #[dialog_common::test]
    async fn it_marks_an_invalid_paste_instead_of_navigating() {
        register();
        let document = window().unwrap().document().unwrap();
        let body = document.body().unwrap();

        let form = document.create_element("form").unwrap();
        let input = document.create_element("input").unwrap();
        input.set_attribute("name", "url").unwrap();
        input
            .set_attribute("value", "definitely not an invite")
            .unwrap();
        let element = document.create_element("tonk-invite-link").unwrap();
        form.append_child(&input).unwrap();
        form.append_child(&element).unwrap();
        body.append_child(&form).unwrap();

        let init = web_sys::EventInit::new();
        init.set_cancelable(true);
        let event = web_sys::Event::new_with_event_init_dict("submit", &init).unwrap();
        let _ = form.dispatch_event(&event);

        assert!(
            event.default_prevented(),
            "the sealed guest must never run a native form submit"
        );
        // The submit resolves on a task; yield until it settles.
        for _ in 0..20 {
            if element.get_attribute(STATE_ATTR).as_deref() == Some("invalid") {
                break;
            }
            gloo_timers::future::TimeoutFuture::new(10).await;
        }
        assert_eq!(
            element.get_attribute(STATE_ATTR).as_deref(),
            Some("invalid"),
        );

        form.remove();
    }

    /// A complete pasted invite reaches the bound command as a `mount`
    /// event carrying `detail.href` — the one read path `tonk:join`
    /// declares. This is the whole reason the form binds `onmount`: a
    /// submit event has no `detail`, so a paste bound to `onsubmit`
    /// wrote a fact under an attribute no handler triggers on and the
    /// button did nothing at all.
    #[dialog_common::test]
    async fn it_hands_a_pasted_invite_to_the_command_as_a_mount_event() {
        register();
        let document = window().unwrap().document().unwrap();
        let body = document.body().unwrap();

        let form = document.create_element("form").unwrap();
        let input = document.create_element("input").unwrap();
        input.set_attribute("name", "url").unwrap();
        input
            .set_attribute(
                "value",
                "https://staging.tonk.xyz/join?access=abc&remote=x#seed",
            )
            .unwrap();
        let element = document.create_element("tonk-invite-link").unwrap();
        form.append_child(&input).unwrap();
        form.append_child(&element).unwrap();
        body.append_child(&form).unwrap();

        // Listen where the binding sits: `mount` must BUBBLE to the form
        // for the display's delegate to see it.
        let seen: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let captured = Rc::clone(&seen);
        let listener = Closure::wrap(Box::new(move |event: Event| {
            let detail = event
                .dyn_ref::<web_sys::CustomEvent>()
                .map(|event| event.detail());
            if let Some(detail) = detail {
                let href = js_sys::Reflect::get(&detail, &JsValue::from_str("href"))
                    .ok()
                    .and_then(|href| href.as_string());
                *captured.borrow_mut() = href;
            }
        }) as Box<dyn FnMut(Event)>);
        form.add_event_listener_with_callback("mount", listener.as_ref().unchecked_ref())
            .unwrap();

        let init = web_sys::EventInit::new();
        init.set_cancelable(true);
        let event = web_sys::Event::new_with_event_init_dict("submit", &init).unwrap();
        let _ = form.dispatch_event(&event);

        assert!(
            event.default_prevented(),
            "the paste is delivered as `mount`, never as a native submit"
        );

        for _ in 0..20 {
            if seen.borrow().is_some() {
                break;
            }
            gloo_timers::future::TimeoutFuture::new(10).await;
        }

        let href = seen.borrow().clone().expect("mount carried no detail.href");
        let url = web_sys::Url::new(&href).unwrap();
        assert_eq!(url.pathname(), "/join", "redeemed on THIS deployment");
        assert_eq!(
            url.search_params().get("access").as_deref(),
            Some("abc"),
            "the delegation chain rides across verbatim"
        );
        assert_eq!(url.hash(), "#seed", "the seed never leaves the browser");

        form.remove();
    }
}
