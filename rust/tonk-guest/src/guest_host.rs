//! Guest-side host installation for the sealed iframe.
//!
//! The guest runs the REAL host IO surface (`tonk_host::install_io`):
//! consumer events bubble to the guest `document`, where the host's
//! own listeners service them over plain `fetch`/SSE. Inside the
//! sealed (opaque-origin) iframe, `window.fetch` is the portal
//! bootstrap's override, which has the outer frame perform each
//! request and stream the response back — so the same transport code
//! serves the top document and every guest, and there is no
//! per-operation envelope relay to miss an event. `window.tonk` is
//! sugar for app code, not the elements' transport.
//!
//! What remains guest-specific here:
//!
//! - the link click relay — the opaque guest can neither move the
//!   parent nor open a tab (no `allow-top-navigation`, no
//!   `allow-popups`), so link clicks post their raw href over
//!   `window.tonk.navigate` or `window.tonk.open` for the host to
//!   resolve and perform;
//! - binding the per-tab `site` entity onto opted-in descendants once
//!   the bridge context arrives.

use std::cell::RefCell;

use js_sys::{Function, Object, Promise, Reflect};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::spawn_local;
use web_sys::{Document, Element, Event, window};

/// Installed listeners, kept alive for the page's lifetime.
type Listener = Closure<dyn FnMut(Event)>;

thread_local! {
    /// The installed navigation listeners. `Some` once [`install`] ran.
    static INSTALLED: RefCell<Option<Vec<Listener>>> = const { RefCell::new(None) };
}

/// Install the guest host: the real IO surface plus the navigation
/// click relay. Idempotent. Also binds the per-tab `site` entity onto
/// opted-in descendants once the bridge context arrives.
pub fn install() {
    // The real host's operation listeners + `with` observer, minus the
    // top-page-only effects (idle-sync heartbeat, navigate provider).
    tonk_host::install_io();

    let already = INSTALLED.with(|cell| cell.borrow().is_some());
    if already {
        return;
    }
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };

    // Navigation relay: a link click inside the opaque guest can't move the
    // parent or open a tab, so catch it at the document and post the href over
    // the bridge for the host to perform. Capture phase so it runs before any
    // app handler and before the (blocked-anyway) native action.
    //
    // `auxclick` as well as `click`: a middle-click does not fire `click` at
    // all, so a `click`-only listener can never see one.
    let mut listeners = Vec::with_capacity(2);
    for event in ["click", "auxclick"] {
        let listener = make_nav_listener(call_bridge);
        let _ = document.add_event_listener_with_callback_and_bool(
            event,
            listener.as_ref().unchecked_ref(),
            true,
        );
        listeners.push(listener);
    }
    INSTALLED.with(|cell| *cell.borrow_mut() = Some(listeners));

    // Bind the per-tab `site` entity: any element that opted in with
    // `data-tonk-entity="site"` gets its `entity` set to the host's site
    // (the X-Tonk-Site the SW keys this tab's `tonk:site` facts on —
    // `window.tonk.context.site`). That is how the routing shell
    // (`<tonk-display model=tonk:site data-tonk-entity="site">`) resolves
    // its own tab's location/route. Deferred until `window.tonk.ready`
    // resolves, since the context (with the site) arrives asynchronously
    // after the host's ready envelope.
    spawn_local(async move {
        await_tonk_ready().await;
        fill_site_entities(&document);
    });
}

/// What a click on a link should do.
#[derive(Debug, PartialEq, Eq)]
enum Intent {
    /// An in-app route change, performed by the host in place.
    Navigate(String),
    /// A new tab, decided and performed by the host. Whether it is announced
    /// first is the HOST's call, not ours — this guest is untrusted, so its
    /// classification is routing, never policy.
    Open(String),
    /// A fragment link, carrying the part after the `#`. Cancelled and
    /// scrolled HERE — see [`classify_click`] for why it is neither native nor
    /// relayed.
    Fragment(String),
    /// `href=""`. Cancelled and warned about; it names no destination.
    Empty,
    /// Not ours. Left to the browser.
    Ignore,
}

/// Build the document click listener that relays link activation to the host.
///
/// `relay` is the bridge call to make — `(method, href)`. Production passes
/// [`call_bridge`]; it is a parameter only so a test can pin WHICH method a
/// given click selects, which is the whole of this function's logic.
fn make_nav_listener(relay: impl Fn(&str, &str) + 'static) -> Listener {
    Closure::wrap(Box::new(move |event: Event| match classify_click(&event) {
        Intent::Ignore => {}
        Intent::Navigate(href) => {
            event.prevent_default();
            relay("navigate", &href);
        }
        Intent::Open(href) => {
            event.prevent_default();
            relay("open", &href);
        }
        Intent::Fragment(fragment) => {
            event.prevent_default();
            scroll_to_fragment(&fragment);
        }
        Intent::Empty => {
            event.prevent_default();
            warn("ignored a link with an empty href: it names no destination");
        }
    }) as Box<dyn FnMut(Event)>)
}

/// Say so when a click produces nothing.
///
/// The guest's console is sanitized out of the parent's, which is why the
/// portal bootstrap relays uncaught errors up over `__tonkRuntime:"warn"`.
/// This is not an error, though — it is a note to whoever authored the view,
/// who is looking at the guest's own console when they look at all.
fn warn(message: &str) {
    web_sys::console::warn_1(&JsValue::from_str(&format!("tonk: {message}")));
}

/// Scroll the guest's OWN document to `fragment` (the part after the `#`),
/// which is the whole of what a fragment link means here. Returns whether
/// there was somewhere to go.
///
/// This is what the browser would have done for a same-document fragment, done
/// by hand because in a `srcdoc` guest it is not one — see [`classify_click`].
///
/// An empty fragment (`href="#"`) and `#top` are the document's top per the
/// HTML spec's "scroll to the fragment" steps, but only when nothing carries
/// that id. An id that matches nothing else scrolls nowhere: the click is
/// still cancelled, because the alternative is loading the app inside the space.
fn scroll_to_fragment(fragment: &str) -> bool {
    let Some(window) = window() else {
        return false;
    };
    let Some(document) = window.document() else {
        return false;
    };
    // The id is a raw URL fragment, so a non-ASCII or spaced id arrives
    // percent-encoded (`#Ünïcode` → `#%C3%9Cn%C3%AFcode`) while the attribute
    // it must match is not. Try the raw form first — an id may legitimately
    // contain a `%` — then the decoded one. A malformed escape simply fails to
    // decode and leaves the raw attempt standing.
    let target = document.get_element_by_id(fragment).or_else(|| {
        js_sys::decode_uri_component(fragment)
            .ok()
            .and_then(|decoded| document.get_element_by_id(&String::from(decoded)))
    });
    if let Some(target) = target {
        target.scroll_into_view();
        return true;
    }
    if fragment.is_empty() || fragment.eq_ignore_ascii_case("top") {
        window.scroll_to_with_x_and_y(0.0, 0.0);
        return true;
    }
    false
}

/// Decide what a click means.
///
/// The guest sets a `<base href="https://{label}.tonk.network/">` to its
/// per-space SYNTHETIC origin, so the BROWSER resolves `anchor.href` for us —
/// a real URL, not the opaque `null/…` it would be with no base. So we
/// classify by ORIGIN, the way a normal page does:
///
/// - same origin as the synthetic base → an in-app route change (`Navigate`);
/// - a different origin → external, handed to the host (`Open`) which decides
///   whether to warn/refuse or open a tab (policy is the host's, never ours);
/// - a link that only changes the fragment → scrolled HERE (`Fragment`), since
///   the guest is the only frame that owns its own document.
///
/// The raw `href` attribute is still read for the empty/pure-fragment cases,
/// which resolution alone can't distinguish from a same-page navigation.
fn classify_click(event: &Event) -> Intent {
    let Some(mouse) = event.dyn_ref::<web_sys::MouseEvent>() else {
        return Intent::Ignore;
    };
    // `auxclick` fires for every non-primary button. Middle (1) means "new
    // tab"; right (2) belongs to the context menu.
    if event.type_() == "auxclick" && mouse.button() != 1 {
        return Intent::Ignore;
    }
    let Some(anchor) = event.target().and_then(closest_anchor) else {
        return Intent::Ignore;
    };
    let Some(href_attr) = anchor.get_attribute("href") else {
        return Intent::Ignore;
    };
    // `href=""` resolves to the bare base — a whole-app reload. It reaches
    // users through a view template whose field hasn't resolved
    // (`<a href="{url}">` renders blank), so it's a live path, not a curiosity.
    if href_attr.is_empty() {
        return Intent::Empty;
    }
    // A pure fragment (`#foo`) addresses THIS document; the guest is the only
    // frame that can honour it. Detect it off the raw attribute (a bare `#…`),
    // before resolution turns it into a full same-path URL.
    if let Some(fragment) = href_attr.strip_prefix('#') {
        return Intent::Fragment(fragment.to_owned());
    }

    // Everything else: let the browser resolve against the synthetic `<base>`.
    // The resolved property is a real URL now (the base is a concrete origin,
    // not `about:srcdoc`).
    let Some(resolved) = anchor
        .dyn_ref::<web_sys::HtmlAnchorElement>()
        .map(|a| a.href())
    else {
        return Intent::Ignore;
    };
    let Ok(url) = web_sys::Url::new(&resolved) else {
        return Intent::Ignore;
    };

    let wants_new_tab = mouse.meta_key()
        || mouse.ctrl_key()
        || mouse.shift_key()
        || mouse.button() == 1
        || anchor
            .get_attribute("target")
            .is_some_and(|target| target == "_blank");

    // Same origin as the coordinate system this guest resolves against →
    // in-app; different → external.
    let in_app = link_origin().is_some_and(|origin| url.origin() == origin);

    if in_app && !wants_new_tab {
        // Relay the guest-world path (the host re-maps it to the real route).
        let mut path = url.pathname();
        path.push_str(&url.search());
        path.push_str(&url.hash());
        Intent::Navigate(path)
    } else {
        Intent::Open(resolved)
    }
}

/// The origin a link in this guest is judged against.
///
/// A space guest resolves against its synthetic per-space base
/// (`https://{label}.tonk.network`), so that is the origin that means "in-app".
/// The profile/Hub guest has no synthetic origin — `space_origin_for` only
/// answers for a `did:key`, and the profile is not one — and its links are
/// genuinely top-level, resolving against the REAL host origin. Judging those
/// against a missing base made every one of them external: the hub's own space
/// links and the FAB's account link were relayed to `open_external`, which
/// opens our own origin in a new tab. Falling back to `context.origin` is what
/// makes a top-level link navigate in place.
///
/// `None` only when the bridge context is absent entirely, where classifying
/// anything as in-app would be a guess.
fn link_origin() -> Option<String> {
    let win: JsValue = window()?.into();
    let tonk = Reflect::get(&win, &JsValue::from_str("tonk")).ok()?;
    let context = Reflect::get(&tonk, &JsValue::from_str("context")).ok()?;
    let field = |name: &str| {
        Reflect::get(&context, &JsValue::from_str(name))
            .ok()
            .and_then(|value| value.as_string())
            .filter(|value| !value.is_empty())
    };
    let origin = field("base").or_else(|| field("origin"))?;
    web_sys::Url::new(&origin).ok().map(|url| url.origin())
}

/// Walk up from an event target to the nearest `<a href>`.
fn closest_anchor(target: web_sys::EventTarget) -> Option<Element> {
    target
        .dyn_into::<Element>()
        .ok()?
        .closest("a[href]")
        .ok()
        .flatten()
}

/// Call a fire-and-forget method on the bridge, if it is installed.
fn call_bridge(method: &str, arg: &str) {
    if let Some(tonk) = window_tonk()
        && let Some(function) = get_fn(&tonk, method)
    {
        let _ = function.call1(&tonk, &JsValue::from_str(arg));
    }
}

/// `window.tonk` if the portal bootstrap installed it.
fn window_tonk() -> Option<Object> {
    let win = window()?;
    Reflect::get(&win, &JsValue::from_str("tonk"))
        .ok()
        .and_then(|v| v.dyn_into::<Object>().ok())
}

/// The host's per-tab `site` entity (`window.tonk.context.site`), the
/// `X-Tonk-Site` the SW keys this tab's `tonk:site` facts on. Lives on `context`
/// (not `tonk` directly) because it is the HOST's id, delivered with the rest of
/// the context in the `ready` envelope.
fn site_id() -> Option<String> {
    let tonk: JsValue = window_tonk()?.into();
    let context = Reflect::get(&tonk, &JsValue::from_str("context")).ok()?;
    Reflect::get(&context, &JsValue::from_str("site"))
        .ok()?
        .as_string()
        .filter(|s| !s.is_empty())
}

/// Await `window.tonk.ready` (resolves once the host's `ready` envelope, with
/// the context, has arrived). Resolves immediately if the bridge is absent.
async fn await_tonk_ready() {
    let Some(tonk) = window_tonk() else {
        return;
    };
    let Ok(ready) = Reflect::get(&tonk, &JsValue::from_str("ready")) else {
        return;
    };
    if let Ok(promise) = ready.dyn_into::<Promise>() {
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }
}

/// Set `entity` to the host's `site` entity on every element in the document
/// that opted in with `data-tonk-entity="site"`. Idempotent: `<tonk-display>`
/// observes `entity`, so a re-set after upgrade just re-resolves to the same
/// value.
fn fill_site_entities(document: &Document) {
    let Some(site) = site_id() else {
        return;
    };
    let Ok(matches) = document.query_selector_all("[data-tonk-entity=\"site\"]") else {
        return;
    };
    for i in 0..matches.length() {
        if let Some(el) = matches.item(i).and_then(|n| n.dyn_into::<Element>().ok()) {
            let _ = el.set_attribute("entity", &site);
        }
    }
}

/// Read `tonk[name]` as a Function.
fn get_fn(tonk: &Object, name: &str) -> Option<Function> {
    Reflect::get(tonk, &JsValue::from_str(name))
        .ok()
        .and_then(|v| v.dyn_into::<Function>().ok())
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use std::rc::Rc;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::{EventInit, EventTarget, MouseEvent, MouseEventInit};

    wasm_bindgen_test_configure!(run_in_browser);

    fn document() -> Document {
        window().expect("a window").document().expect("a document")
    }

    /// The test document's own origin — the synthetic per-space base the
    /// classifier resolves against in these tests. Detached anchors resolve
    /// their `.href` against this document's base, so pinning
    /// `window.tonk.context.base` to the same origin makes an in-space path
    /// (`/space/abc`) classify as same-origin `Navigate`, exactly as a real
    /// guest whose `<base>` is its per-space origin.
    fn test_origin() -> String {
        window().unwrap().location().origin().unwrap()
    }

    /// The guest bridge context, creating `window.tonk.context` if absent.
    fn test_context() -> JsValue {
        let win: JsValue = window().unwrap().into();
        let tonk = match Reflect::get(&win, &JsValue::from_str("tonk")) {
            Ok(v) if v.is_object() => v,
            _ => {
                let o = Object::new();
                let _ = Reflect::set(&win, &JsValue::from_str("tonk"), &o);
                o.into()
            }
        };
        match Reflect::get(&tonk, &JsValue::from_str("context")) {
            Ok(v) if v.is_object() => v,
            _ => {
                let o = Object::new();
                let _ = Reflect::set(&tonk, &JsValue::from_str("context"), &o);
                o.into()
            }
        }
    }

    fn set_context_field(name: &str, value: &str) {
        let _ = Reflect::set(
            &test_context(),
            &JsValue::from_str(name),
            &JsValue::from_str(value),
        );
    }

    /// Install `window.tonk.context.base = <origin>/` so [`link_origin`] has a
    /// synthetic base to compare against. Idempotent; every classify test that
    /// depends on origin comparison calls it first.
    fn set_test_base() {
        set_context_field("base", &format!("{}/", test_origin()));
    }

    /// Put the context in the shape a PROFILE guest gets: no synthetic space
    /// origin (`space_origin_for` only answers for a `did:key`), just the real
    /// host origin.
    fn set_profile_context() {
        set_context_field("base", "");
        set_context_field("origin", &test_origin());
    }

    /// A detached `<a>` carrying the given attributes.
    ///
    /// Detached on purpose: `classify_click` only ever reads the event and the
    /// anchor's attributes, so nothing here touches the shared document — these
    /// tests cannot leak state into each other or into a later test.
    fn anchor(attributes: &[(&str, &str)]) -> Element {
        let anchor = document().create_element("a").expect("an anchor");
        for (name, value) in attributes {
            anchor.set_attribute(name, value).expect("set attribute");
        }
        anchor
    }

    /// The mouse state of a click, as the browser would deliver it.
    struct Click {
        kind: &'static str,
        button: i16,
        meta: bool,
        ctrl: bool,
        shift: bool,
    }

    impl Click {
        /// An unmodified primary click — the plain in-app case.
        fn plain() -> Self {
            Self {
                kind: "click",
                button: 0,
                meta: false,
                ctrl: false,
                shift: false,
            }
        }

        /// A non-primary press. `auxclick` is the event the browser actually
        /// fires for these; `click` never sees them.
        fn aux(button: i16) -> Self {
            Self {
                kind: "auxclick",
                button,
                ..Self::plain()
            }
        }
    }

    /// Dispatch `event` at `target` and return how `classify_click` reads it.
    ///
    /// The classification happens inside a listener because `event.target()` is
    /// only populated for the duration of a dispatch — reading it off a
    /// constructed event would see `None` and prove nothing.
    ///
    /// The listener also cancels the event, which is not incidental: an `<a>`
    /// follows its hyperlink even while DETACHED (the spec only requires a
    /// connected node for elements that aren't `<a>`), so an uncancelled click
    /// here navigates the test page away mid-run and a `javascript:` href
    /// executes. Production cancels for the same reason.
    fn dispatch(target: &EventTarget, kind: &str, event: &Event) -> Intent {
        // `Option`, not an `Intent::Ignore` sentinel: a listener that never ran
        // must be a message, not a value indistinguishable from a real `Ignore`.
        let seen = Rc::new(RefCell::new(None));
        let sink = Rc::clone(&seen);
        let listener = Closure::wrap(Box::new(move |event: Event| {
            *sink.borrow_mut() = Some(classify_click(&event));
            event.prevent_default();
        }) as Box<dyn FnMut(Event)>);
        with_listener(target, kind, &listener, || {
            target.dispatch_event(event).expect("dispatch");
        });
        seen.borrow_mut().take().expect("the listener ran")
    }

    /// Register `listener` on `target` for the duration of `body`, then remove
    /// it. A closure dropped while still registered leaves the target calling
    /// into freed memory ("closure invoked after being dropped") the moment any
    /// later event reaches it.
    fn with_listener(target: &EventTarget, kind: &str, listener: &Listener, body: impl FnOnce()) {
        let callback = listener.as_ref().unchecked_ref();
        target
            .add_event_listener_with_callback(kind, callback)
            .expect("add listener");
        body();
        target
            .remove_event_listener_with_callback(kind, callback)
            .expect("remove listener");
    }

    /// A real `MouseEvent` carrying `click`'s button and modifier state, as the
    /// browser would deliver it.
    fn mouse_event(click: &Click) -> MouseEvent {
        let init = MouseEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_button(click.button);
        init.set_meta_key(click.meta);
        init.set_ctrl_key(click.ctrl);
        init.set_shift_key(click.shift);
        MouseEvent::new_with_mouse_event_init_dict(click.kind, &init).expect("a mouse event")
    }

    fn classify(target: &EventTarget, click: &Click) -> Intent {
        set_test_base();
        dispatch(target, click.kind, &mouse_event(click))
    }

    /// Classify against whatever context the caller has already installed,
    /// rather than resetting it to a space guest's synthetic base.
    fn classify_in_context(target: &EventTarget, click: &Click) -> Intent {
        dispatch(target, click.kind, &mouse_event(click))
    }

    /// Classify a click on an `<a href=…>` with no other attributes.
    fn classify_href(href: &str, click: &Click) -> Intent {
        classify(anchor(&[("href", href)]).unchecked_ref(), click)
    }

    /// The absolute URL a detached anchor's `href="…"` resolves to, against
    /// the test document's base — what the classifier reports for an `Open`.
    fn resolved(href: &str) -> String {
        let a = anchor(&[("href", href)]);
        a.unchecked_ref::<web_sys::HtmlAnchorElement>().href()
    }

    /// What the production listener did with a click: the bridge call it
    /// relayed, if any, and whether it cancelled the event.
    struct Relayed {
        call: Option<(String, String)>,
        cancelled: bool,
    }

    /// Dispatch `click` at `target` through the REAL [`make_nav_listener`],
    /// recording the bridge call it selects.
    ///
    /// This is what pins the arm mapping. `classify_click` returning
    /// `Navigate` says nothing about which bridge method the listener then
    /// calls with it — swapping the two arms inverts every link in the app and
    /// no classifier test can see it. Driving the production closure and
    /// recording the method NAME is the only assertion that can.
    fn relay(target: &EventTarget, click: &Click) -> Relayed {
        set_test_base();
        let calls = Rc::new(RefCell::new(None));
        let sink = Rc::clone(&calls);
        let listener = make_nav_listener(move |method: &str, arg: &str| {
            *sink.borrow_mut() = Some((method.to_owned(), arg.to_owned()));
        });
        let mut cancelled = false;
        with_listener(target, click.kind, &listener, || {
            // `dispatch_event` reports the event was NOT cancelled, which is
            // exactly `preventDefault` having gone uncalled.
            cancelled = !target
                .dispatch_event(&mouse_event(click))
                .expect("dispatch");
        });
        Relayed {
            call: calls.borrow_mut().take(),
            cancelled,
        }
    }

    /// The mapping the whole change exists for: a plain in-app click reaches
    /// the host's ROUTER, not its new-tab path.
    #[dialog_common::test]
    async fn it_relays_a_plain_in_app_click_to_navigate() {
        let relayed = relay(
            anchor(&[("href", "/space/abc")]).unchecked_ref(),
            &Click::plain(),
        );
        assert_eq!(
            relayed.call,
            Some(("navigate".to_owned(), "/space/abc".to_owned())),
            "an in-app click should call the bridge's `navigate` with the raw href"
        );
        assert!(
            relayed.cancelled,
            "the relayed click must be cancelled, or the guest follows the link too"
        );
    }

    /// The other half of the mapping: a new-tab click reaches the host's `open`
    /// and must never be handed to the router as a route.
    #[dialog_common::test]
    async fn it_relays_a_new_tab_click_to_open() {
        let click = Click {
            meta: true,
            ..Click::plain()
        };
        let relayed = relay(
            anchor(&[("href", "https://example.com/x")]).unchecked_ref(),
            &click,
        );
        assert_eq!(
            relayed.call,
            Some(("open".to_owned(), "https://example.com/x".to_owned())),
            "a new-tab click should call the bridge's `open` with the raw href"
        );
        assert!(
            relayed.cancelled,
            "the relayed click must be cancelled, or the guest follows the link too"
        );
    }

    /// An ignored click is not the host's business at all: no bridge call, and
    /// the event left alone for the browser to act on.
    #[dialog_common::test]
    async fn it_relays_nothing_for_an_ignored_click() {
        let bare = document().create_element("div").expect("a div");
        let relayed = relay(bare.unchecked_ref(), &Click::plain());
        assert_eq!(
            relayed.call, None,
            "a click with no link to follow should reach the bridge not at all"
        );
        assert!(
            !relayed.cancelled,
            "an ignored click must keep its default action; the browser owns it"
        );
    }

    /// A plain click on an in-app path is the one case the host performs in
    /// place, without a new tab.
    #[dialog_common::test]
    async fn it_navigates_a_plain_in_app_click() {
        assert_eq!(
            classify_href("/space/abc", &Click::plain()),
            Intent::Navigate("/space/abc".to_owned()),
            "a plain click on a path should navigate in place"
        );
    }

    /// The anchor is rarely the event target — the click lands on whatever is
    /// inside it, so classification has to walk up to the nearest `<a href>`.
    #[dialog_common::test]
    async fn it_classifies_a_click_on_a_descendant_of_the_anchor() {
        let anchor = anchor(&[("href", "/space/abc")]);
        let label = document().create_element("span").expect("a span");
        anchor.append_child(&label).expect("nest the span");

        assert_eq!(
            classify(label.unchecked_ref(), &Click::plain()),
            Intent::Navigate("/space/abc".to_owned()),
            "a click on a child of the anchor should classify by the anchor"
        );
    }

    /// Each modifier the browser treats as "open in a new tab" routes to the
    /// host's `open`, even for an in-app path the guest could otherwise route
    /// in place.
    #[dialog_common::test]
    async fn it_opens_an_in_app_path_on_a_modified_click() {
        let modified = [
            (
                "meta",
                Click {
                    meta: true,
                    ..Click::plain()
                },
            ),
            (
                "ctrl",
                Click {
                    ctrl: true,
                    ..Click::plain()
                },
            ),
            (
                "shift",
                Click {
                    shift: true,
                    ..Click::plain()
                },
            ),
            ("middle", Click::aux(1)),
        ];
        for (name, click) in &modified {
            assert_eq!(
                classify_href("/space/abc", click),
                Intent::Open(resolved("/space/abc")),
                "a {name} click should open rather than navigate in place"
            );
        }
    }

    /// The profile guest — the Hub, and the space chrome that mounts the FAB —
    /// has no synthetic space origin, because `space_origin_for` only answers
    /// for a `did:key`. Its links are top-level and resolve against the real
    /// host origin, so they must navigate in place.
    ///
    /// Judged against the missing base instead, every one of them was
    /// `Open` — relayed to `open_external`, which opens our own origin with
    /// `window.open(_, "_blank")`. That is what put `/settings` in a new tab.
    #[dialog_common::test]
    async fn it_navigates_a_top_level_link_in_the_profile_guest() {
        set_profile_context();
        for href in [
            "/settings",
            "/settings?next=%2Fspace%2Fabc",
            "/space/abc",
            "/",
        ] {
            assert_eq!(
                classify_in_context(anchor(&[("href", href)]).unchecked_ref(), &Click::plain()),
                Intent::Navigate(href.to_owned()),
                "{href} addresses the host origin and should navigate in place"
            );
        }
    }

    /// The fallback is the REAL origin, not "anything goes": an off-origin
    /// link in the profile guest still leaves through the host, where the
    /// confirmation dialog lives.
    #[dialog_common::test]
    async fn it_still_opens_an_off_origin_link_in_the_profile_guest() {
        set_profile_context();
        assert_eq!(
            classify_in_context(
                anchor(&[("href", "https://example.test/docs")]).unchecked_ref(),
                &Click::plain(),
            ),
            Intent::Open("https://example.test/docs".to_owned()),
        );
    }

    /// `target="_blank"` asks for a new tab without any modifier.
    #[dialog_common::test]
    async fn it_opens_a_target_blank_link() {
        set_test_base();
        assert_eq!(
            classify(
                anchor(&[("href", "/space/abc"), ("target", "_blank")]).unchecked_ref(),
                &Click::plain(),
            ),
            Intent::Open(resolved("/space/abc")),
            "target=_blank should open a new tab"
        );
        assert_eq!(
            classify(
                anchor(&[("href", "/space/abc"), ("target", "_self")]).unchecked_ref(),
                &Click::plain(),
            ),
            Intent::Navigate("/space/abc".to_owned()),
            "only _blank asks for a new tab; _self should navigate in place"
        );
    }

    /// `auxclick` fires for every non-primary button, so the classifier must
    /// read the button: middle means "new tab", right belongs to the context
    /// menu and must be left entirely alone.
    #[dialog_common::test]
    async fn it_ignores_a_right_click() {
        assert_eq!(
            classify_href("/space/abc", &Click::aux(2)),
            Intent::Ignore,
            "a right click should be left to the context menu"
        );
    }

    /// The event type, not just the button, decides: the two listeners share one
    /// classifier, so a `click` and an `auxclick` carrying identical button
    /// state must still be told apart.
    #[dialog_common::test]
    async fn it_distinguishes_click_from_auxclick() {
        let primary_aux = Click {
            kind: "auxclick",
            ..Click::plain()
        };
        assert_eq!(
            classify_href("/space/abc", &primary_aux),
            Intent::Ignore,
            "auxclick with the primary button is not a middle click"
        );
        assert_eq!(
            classify_href("/space/abc", &Click::plain()),
            Intent::Navigate("/space/abc".to_owned()),
            "the same button state on a click should navigate"
        );
    }

    /// A fragment addresses the GUEST's document, so it is neither the host's
    /// business nor the browser's: the guest is the only frame that can honour
    /// it, and the only one that must cancel it.
    #[dialog_common::test]
    async fn it_classifies_a_fragment_as_the_guest_s_own_scroll() {
        assert_eq!(
            classify_href("#section", &Click::plain()),
            Intent::Fragment("section".to_owned()),
            "a fragment should be scrolled to inside the guest"
        );
        assert_eq!(
            classify_href("#", &Click::plain()),
            Intent::Fragment(String::new()),
            "a bare `#` is the top of the guest's document"
        );
    }

    /// The fragment row beats the modifier row. A cmd-clicked `#x` is not a new
    /// tab, even though a browser would make one: the host would resolve `#x`
    /// against `/space/{id}` and open a duplicate space scrolled nowhere.
    #[dialog_common::test]
    async fn it_classifies_a_modified_fragment_click_as_a_fragment() {
        let click = Click {
            meta: true,
            ..Click::plain()
        };
        assert_eq!(
            classify_href("#section", &click),
            Intent::Fragment("section".to_owned()),
            "a modified fragment click is still the guest's own scroll"
        );
    }

    /// A fragment is NOT same-document in a `srcdoc` guest: the document URL is
    /// `about:srcdoc` but the BASE URL is inherited from the parent, so `#foo`
    /// resolves to `https://origin/space/{id}#foo` — a different document. Left
    /// native, the browser loads the whole app inside the space's iframe.
    #[dialog_common::test]
    async fn it_cancels_a_fragment_click_rather_than_letting_it_navigate() {
        let relayed = relay(
            anchor(&[("href", "#section")]).unchecked_ref(),
            &Click::plain(),
        );
        assert!(
            relayed.cancelled,
            "a fragment click must be cancelled, or the guest navigates to the parent's URL"
        );
        assert_eq!(
            relayed.call, None,
            "a fragment addresses the guest's own document — the host must not be involved"
        );
    }

    /// `href=""` resolves to the BARE parent URL, which is the same whole-app
    /// load one hop shorter. A blank field in a view template (`<a href="{url}">`
    /// before the field resolves) is the way this reaches a user.
    #[dialog_common::test]
    async fn it_cancels_an_empty_href_click() {
        let relayed = relay(anchor(&[("href", "")]).unchecked_ref(), &Click::plain());
        assert!(
            relayed.cancelled,
            "an empty href must be cancelled, or the guest reloads the app inside itself"
        );
        assert_eq!(
            relayed.call, None,
            "an empty href names no destination — nothing to relay"
        );
    }

    /// Anything that resolves to a DIFFERENT origin than the guest's synthetic
    /// base goes to the host to open. The relayed href is the browser-resolved
    /// absolute URL (a protocol-relative `//host` becomes `https://host`).
    #[dialog_common::test]
    async fn it_opens_an_off_origin_link() {
        set_test_base();
        for href in ["https://example.com/x", "//example.com/x"] {
            assert_eq!(
                classify_href(href, &Click::plain()),
                Intent::Open(resolved(href)),
                "{href} resolves off-origin and should be opened by the host"
            );
        }
    }

    /// Non-http schemes are relayed too, INCLUDING ones the host is expected to
    /// refuse. The guest classifies for routing, never for policy: a component
    /// can call `window.tonk.open` directly, so filtering here would buy no
    /// safety while turning the host's console warning back into a silently
    /// dead click.
    #[dialog_common::test]
    async fn it_relays_schemes_the_host_is_expected_to_refuse() {
        set_test_base();
        for href in ["mailto:a@example.com", "tel:+1234", "javascript:alert(1)"] {
            assert_eq!(
                classify_href(href, &Click::plain()),
                Intent::Open(resolved(href)),
                "{href} should reach the host, which is the one that decides"
            );
        }
    }

    /// A click with no anchor above it is not ours. An anchor with an EMPTY
    /// href is: left alone it reloads the app inside the space.
    #[dialog_common::test]
    async fn it_ignores_a_click_with_no_link_to_follow() {
        let bare = document().create_element("div").expect("a div");
        assert_eq!(
            classify(bare.unchecked_ref(), &Click::plain()),
            Intent::Ignore,
            "a click outside any anchor should be ignored"
        );
        assert_eq!(
            classify_href("", &Click::plain()),
            Intent::Empty,
            "an empty href names nothing, but must still be cancelled"
        );
    }

    /// The scroll a fragment click stands in for. `href="#x"` means "put the
    /// element with id `x` in view", and in a `srcdoc` guest nothing else will
    /// do it.
    #[dialog_common::test]
    async fn it_scrolls_the_guest_to_a_fragment_target() {
        let document = document();
        let body = document.body().expect("a body");
        let target = document.create_element("div").expect("a div");
        target
            .set_attribute("id", "tonk-test-fragment-target")
            .expect("set the id");
        body.append_child(&target).expect("attach the target");

        let found = scroll_to_fragment("tonk-test-fragment-target");
        let top = scroll_to_fragment("");
        let missing = scroll_to_fragment("tonk-test-no-such-id");

        // Leave the shared document as it was found, before any assertion can
        // unwind past the cleanup.
        target.remove();

        assert!(found, "an id that exists should be scrolled into view");
        assert!(top, "an empty fragment is the top of the document");
        assert!(
            !missing,
            "an id that matches nothing has nowhere to scroll to"
        );
    }

    /// A percent-encoded fragment must still find its element: the href is a
    /// URL, the `id` attribute is not, so `#%C3%9C` and `id="Ü"` are the same
    /// target spelled two ways.
    #[dialog_common::test]
    async fn it_scrolls_to_a_percent_encoded_fragment_target() {
        let document = document();
        let body = document.body().expect("a body");
        let target = document.create_element("div").expect("a div");
        target
            .set_attribute("id", "tonk-test-\u{00DC}")
            .expect("set the id");
        body.append_child(&target).expect("attach the target");

        let found = scroll_to_fragment("tonk-test-%C3%9C");

        target.remove();

        assert!(found, "a percent-encoded fragment should find its element");
    }

    /// A non-mouse event reaching the listener must not be classified as a
    /// click: the modifier and button state that routing depends on simply
    /// isn't there to read.
    #[dialog_common::test]
    async fn it_ignores_a_non_mouse_event() {
        let init = EventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        let event = Event::new_with_event_init_dict("click", &init).expect("a plain event");

        assert_eq!(
            dispatch(
                anchor(&[("href", "/space/abc")]).unchecked_ref(),
                "click",
                &event
            ),
            Intent::Ignore,
            "a plain Event is not a MouseEvent and should be ignored"
        );
    }

    /// The reason the `auxclick` listener exists at all: this browser fires a
    /// distinct `auxclick` event for non-primary buttons.
    ///
    /// This pins that the event is real here, which is what makes registering
    /// the second listener meaningful. It does NOT prove the stronger claim
    /// that a native middle-click fires `auxclick` and no `click` — a synthetic
    /// dispatch cannot show that, since it fires whatever event it is told to.
    /// That half is browser-spec behaviour, checked by hand in the browser pass.
    #[dialog_common::test]
    async fn it_runs_where_auxclick_exists() {
        let window = window().expect("a window");
        assert!(
            Reflect::has(&window, &JsValue::from_str("onauxclick")).expect("reflect has"),
            "auxclick should be supported, or the second listener is dead code"
        );
    }
}
