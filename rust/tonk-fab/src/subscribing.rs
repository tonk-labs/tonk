//! Shared scaffolding for the FAB's subscribing `ui-` children.
//!
//! `<ui-space-name>`, `<ui-member-roster>`, and (soon) `<ui-space-switcher>`
//! all: render with `shadow() -> false`, observe a `space` attribute, stamp
//! their OWN `with="main@{did}"` on connect (`resolve_with` reads an
//! element's own attribute and never walks ancestors, so each element must
//! stamp it itself, not inherit it), subscribe via plain `consumer::subscribe`
//! (not `subscribe_with_route` — that has one caller, the portal bridge, and
//! is not this precedent), retry a failed subscribe with a bounded
//! `RetryPolicy` before giving up to a terminal `data-state="unavailable"`,
//! and install `reset`/`update` frame delegates so the host's delivered
//! frames are actually consumed.
//!
//! That last point is load-bearing: `<ui-space-name>` originally subscribed
//! but never wired a delegate to consume the answer, so it silently showed
//! "Untitled" forever (see commit 71d1c58ac). Routing both frame kinds
//! through [`Subscribing::render_reset`]/[`Subscribing::render_update`] here
//! makes consumption structural — an element built on this scaffolding
//! cannot subscribe without also rendering.
//!
//! What elements differ on is the query body and how a frame renders, so
//! that is exactly the seam [`Subscribing`] exposes.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use js_sys::{Function, JSON, Reflect};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, HtmlElement, window};

use tonk_host::consumer::{self, Subscription};

use crate::retry::RetryPolicy;

/// The per-instance frame-delegate closure shape: `(payload, opts)`, matching
/// what the host's `invoke_method_marked` calls with.
type FrameClosure = Closure<dyn FnMut(JsValue, JsValue)>;

/// The per-element behaviour a subscribing `ui-` child supplies; the
/// scaffolding around it (with-stamping, subscribe, retry, teardown) is
/// shared.
///
/// `render_reset`/`render_update` are split, not a single `render`, because
/// the two frame kinds carry different shapes: `reset` delivers a bare array
/// of every current conclusion, `update` delivers `{ asserted, retracted }`
/// deltas — `<ui-space-name>` already needed to tell them apart (a delta's
/// newest asserted row wins; a bare retract leaves the chip alone), so the
/// scaffolding names both rather than pushing that dispatch into every
/// implementer.
///
/// `resolve_with`/`query_body` both take the host element rather than a
/// pre-extracted `space` string because implementers disagree on where their
/// routing context comes from: `<ui-space-name>`/`<ui-member-roster>` derive
/// `main@{did}` from their own `space` attribute (the default
/// [`Subscribing::resolve_with`] below), but `<ui-space-switcher>` reads the
/// PROFILE branch — its `with` is the fixed literal `"main@profile:tonk"`,
/// not derived from any attribute at all. Handing the element to the
/// implementer, rather than hard-assuming a `space`-shaped attribute in the
/// scaffolding, is what lets both coexist.
pub trait Subscribing {
    /// Resolve the `with` routing context to stamp on the element and
    /// subscribe through. `None` means the context isn't ready yet (an
    /// unsubstituted `{id}` placeholder, say) — the attribute-changed
    /// callback re-runs [`Scaffold::connect`] once it lands.
    ///
    /// Default: read this element's own `space` attribute and map it through
    /// [`crate::logic::space_with`] (`main@{did}`) — the shape
    /// `<ui-space-name>` and `<ui-member-roster>` both use. An implementer
    /// whose routing context isn't space-derived (`<ui-space-switcher>`)
    /// overrides this.
    fn resolve_with(&self, this: &HtmlElement) -> Option<String> {
        this.get_attribute("space")
            .filter(|s| !s.is_empty())
            .map(|space| crate::logic::space_with(&space))
    }
    /// The subscribe body. Reads whatever attribute(s) on `this` the query
    /// needs — `<ui-space-name>` reads `space` for the subject it binds
    /// `this` to, `<ui-member-roster>`/`<ui-space-switcher>` ignore `this`
    /// entirely (directory-mode queries with no subject to bind).
    fn query_body(&self, this: &HtmlElement) -> Result<String, String>;
    /// Render a full snapshot (`reset`) frame into the host.
    fn render_reset(&self, host: &HtmlElement, payload: &JsValue);
    /// Render an incremental (`update`) delta frame into the host.
    fn render_update(&self, host: &HtmlElement, payload: &JsValue);
    /// Tag distinguishing this element's subscription — also used as the log
    /// prefix on a failed subscribe.
    fn tag(&self) -> &'static str;
}

/// The shared subscribe/retry/teardown state a subscribing element's
/// `CustomElement` struct embeds alongside its own fields.
///
/// An element may hold SEVERAL subscriptions. The host delivers every frame
/// to the same `reset`/`update` methods, so they are told apart by the `tag`
/// in the frame's options — the same tag each behaviour supplied when it
/// subscribed. `<tonk-share>` needs this: its invite link and its refusal
/// signal are separate inline predicates over different raw attributes, and a
/// single predicate over both would resolve only when BOTH are present, which
/// is never.
#[derive(Default)]
pub struct Scaffold {
    /// Live subscriptions, paired with the tag they were opened under.
    subscriptions: Rc<RefCell<Vec<(String, Subscription)>>>,
    reset: Rc<RefCell<Option<FrameClosure>>>,
    update: Rc<RefCell<Option<FrameClosure>>>,
    /// Invalidates delayed subscribe attempts when the element disconnects.
    generation: Rc<Cell<u64>>,
}

impl Scaffold {
    /// Run from `connected_callback` with a single behaviour. See
    /// [`Self::connect_all`].
    pub fn connect(&self, this: &HtmlElement, behaviour: Rc<dyn Subscribing>) {
        self.connect_all(this, vec![behaviour]);
    }

    /// Run from `connected_callback`: stamp `with`, install the `reset`/
    /// `update` frame delegates (forwarded from the prototype shims
    /// [`install_frame_shims`] installs), and subscribe each behaviour under
    /// its own tag.
    ///
    /// The routing context comes from the FIRST behaviour — every behaviour on
    /// one element shares that element's `with`. A no-op when it returns
    /// `None` (context not ready yet); the attribute-changed callback re-runs
    /// this once it lands.
    pub fn connect_all(&self, this: &HtmlElement, behaviours: Vec<Rc<dyn Subscribing>>) {
        let Some(first) = behaviours.first() else {
            return;
        };
        let Some(with) = first.resolve_with(this) else {
            return;
        };
        let _ = this.set_attribute("with", &with);

        let routed = behaviours.clone();
        let host = this.clone();
        let reset: FrameClosure =
            Closure::wrap(Box::new(move |payload: JsValue, opts: JsValue| {
                if let Some(behaviour) = route(&routed, &opts) {
                    behaviour.render_reset(&host, &payload);
                }
            }));
        let _ = Reflect::set(this, &"__tonkReset".into(), reset.as_ref());
        *self.reset.borrow_mut() = Some(reset);

        let routed = behaviours.clone();
        let host = this.clone();
        let update: FrameClosure =
            Closure::wrap(Box::new(move |payload: JsValue, opts: JsValue| {
                if let Some(behaviour) = route(&routed, &opts) {
                    behaviour.render_update(&host, &payload);
                }
            }));
        let _ = Reflect::set(this, &"__tonkUpdate".into(), update.as_ref());
        *self.update.borrow_mut() = Some(update);

        for behaviour in behaviours {
            let subscriptions = self.subscriptions.clone();
            let host = this.clone();
            let generation = self.generation.clone();
            let expected_generation = generation.get();
            // Each behaviour gets its own retry budget: one query failing to
            // build must not spend the other's attempts.
            let retry = Rc::new(RefCell::new(RetryPolicy::default()));
            spawn_local(async move {
                let tag = behaviour.tag().to_owned();
                if !host.is_connected()
                    || generation.get() != expected_generation
                    || subscriptions.borrow().iter().any(|(open, _)| *open == tag)
                {
                    return;
                }
                subscribe(
                    host,
                    behaviour,
                    subscriptions,
                    retry,
                    generation,
                    expected_generation,
                );
            });
        }
    }

    /// Run from `disconnected_callback`: drop every subscription and the frame
    /// delegates.
    pub fn disconnect(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        self.subscriptions.borrow_mut().clear();
        self.reset.borrow_mut().take();
        self.update.borrow_mut().take();
    }
}

/// Pick the behaviour a frame belongs to by the `tag` in its options — the
/// tag that behaviour supplied when it subscribed.
///
/// With exactly one behaviour, an absent or unrecognised tag still routes to
/// it: single-subscription elements predate tagged routing and must keep
/// working whether or not the host echoes a tag.
///
/// A frame that matches nothing (only possible with more than one behaviour)
/// is dropped, not misdelivered to an arbitrary one — logged so a future
/// host/element protocol mismatch is debuggable rather than silently inert.
fn route<'a>(
    behaviours: &'a [Rc<dyn Subscribing>],
    opts: &JsValue,
) -> Option<&'a Rc<dyn Subscribing>> {
    let tag = Reflect::get(opts, &"tag".into())
        .ok()
        .and_then(|value| value.as_string());
    let routed = match &tag {
        Some(tag) => behaviours
            .iter()
            .find(|behaviour| behaviour.tag() == tag)
            .or_else(|| (behaviours.len() == 1).then(|| &behaviours[0])),
        None => (behaviours.len() == 1).then(|| &behaviours[0]),
    };
    if routed.is_none() {
        tonk_common::log!(
            "frame dropped: tag {tag:?} matched none of {} behaviour(s)",
            behaviours.len()
        );
    }
    routed
}

fn subscribe(
    host: HtmlElement,
    behaviour: Rc<dyn Subscribing>,
    subscriptions: Rc<RefCell<Vec<(String, Subscription)>>>,
    retry: Rc<RefCell<RetryPolicy>>,
    generation: Rc<Cell<u64>>,
    expected_generation: u64,
) {
    let tag = behaviour.tag();
    if !host.is_connected()
        || generation.get() != expected_generation
        || subscriptions.borrow().iter().any(|(open, _)| open == tag)
    {
        return;
    }
    let body = match behaviour.query_body(&host) {
        Ok(body) => body,
        Err(err) => {
            tonk_common::log!("{tag}: query build failed: {err}");
            return;
        }
    };
    let Ok(parsed) = JSON::parse(&body) else {
        tonk_common::log!("{tag}: query JSON parse failed");
        return;
    };
    let consumer_el: Element = host.clone().into();
    let tag_val = JsValue::from_str(tag);
    match consumer::subscribe(&consumer_el, &parsed, Some(&tag_val)) {
        Ok(sub) => {
            if !host.is_connected() || generation.get() != expected_generation {
                return;
            }
            retry.borrow_mut().reset();
            let mut subscriptions = subscriptions.borrow_mut();
            if !subscriptions.iter().any(|(open, _)| open == tag) {
                subscriptions.push((tag.to_owned(), sub));
            }
        }
        Err(err) => {
            // Bounded, unlike the host's default resubscribe loop.
            let delay = retry.borrow_mut().next_delay_ms();
            match delay {
                Some(delay) => {
                    tonk_common::log!("{tag}: subscribe failed, will retry: {err:?}");
                    spawn_local(async move {
                        wait_ms(delay).await;
                        subscribe(
                            host,
                            behaviour,
                            subscriptions,
                            retry,
                            generation,
                            expected_generation,
                        );
                    });
                }
                None => {
                    tonk_common::log!("{tag}: subscribe failed, giving up: {err:?}");
                    if host.is_connected() && generation.get() == expected_generation {
                        let _ = host.set_attribute("data-state", "unavailable");
                    }
                }
            }
        }
    }
}

async fn wait_ms(ms: i32) {
    let Some(win) = window() else {
        return;
    };
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if win
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .is_err()
        {
            let _ = resolve.call0(&JsValue::UNDEFINED);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// Whether `tag` is already a registered custom element.
pub fn already_registered(tag: &str) -> bool {
    window()
        .map(|win| !win.custom_elements().get(tag).is_undefined())
        .unwrap_or(false)
}

/// Install the prototype `reset`/`update` method shims (forwarding to the
/// per-instance `__tonkReset`/`__tonkUpdate` delegates [`Scaffold::connect`]
/// installs) so host subscription frames reach the element — the same
/// pattern `<ui-sync-status>` uses.
pub fn install_frame_shims(tag: &str) {
    let Some(win) = window() else {
        return;
    };
    let constructor = win.custom_elements().get(tag);
    if constructor.is_undefined() {
        return;
    }
    let Ok(proto) = Reflect::get(&constructor, &"prototype".into()) else {
        return;
    };
    let reset_fn = Function::new_with_args(
        "payload, opts",
        "if (typeof this.__tonkReset === 'function') this.__tonkReset(payload, opts);",
    );
    let _ = Reflect::set(&proto, &"reset".into(), &reset_fn);
    let update_fn = Function::new_with_args(
        "payload, opts",
        "if (typeof this.__tonkUpdate === 'function') this.__tonkUpdate(payload, opts);",
    );
    let _ = Reflect::set(&proto, &"update".into(), &update_fn);
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use js_sys::Promise;
    use std::cell::Cell;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::CustomEvent;
    wasm_bindgen_test_configure!(run_in_browser);

    use std::cell::RefCell;
    use std::rc::Rc;

    async fn wait_ms(ms: i32) {
        let promise = Promise::new(&mut |resolve, _| {
            window()
                .expect("window")
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
                .expect("timeout");
        });
        wasm_bindgen_futures::JsFuture::from(promise)
            .await
            .expect("timeout resolves");
    }

    /// A behaviour that records which payloads it was handed.
    struct Recorder {
        tag: &'static str,
        seen: Rc<RefCell<Vec<String>>>,
    }

    impl Subscribing for Recorder {
        fn query_body(&self, _this: &HtmlElement) -> Result<String, String> {
            Ok("{}".to_owned())
        }
        fn render_reset(&self, _host: &HtmlElement, payload: &JsValue) {
            self.seen
                .borrow_mut()
                .push(payload.as_string().unwrap_or_default());
        }
        fn render_update(&self, _host: &HtmlElement, _payload: &JsValue) {}
        fn tag(&self) -> &'static str {
            self.tag
        }
    }

    /// A frame tagged for one behaviour reaches only that behaviour.
    #[dialog_common::test]
    fn it_routes_frames_by_tag() {
        let document = window().unwrap().document().unwrap();
        let host: HtmlElement = document.create_element("div").unwrap().dyn_into().unwrap();
        host.set_attribute("space", "did:key:z6Mk").unwrap();

        let first = Rc::new(RefCell::new(Vec::new()));
        let second = Rc::new(RefCell::new(Vec::new()));
        let scaffold = Scaffold::default();
        scaffold.connect_all(
            &host,
            vec![
                Rc::new(Recorder {
                    tag: "one",
                    seen: Rc::clone(&first),
                }),
                Rc::new(Recorder {
                    tag: "two",
                    seen: Rc::clone(&second),
                }),
            ],
        );

        // Deliver a frame the way the host does: element.__tonkReset(payload, {tag}).
        let opts = js_sys::Object::new();
        Reflect::set(&opts, &"tag".into(), &"two".into()).unwrap();
        let reset = Reflect::get(&host, &"__tonkReset".into())
            .unwrap()
            .dyn_into::<Function>()
            .unwrap();
        reset
            .call2(&JsValue::NULL, &JsValue::from_str("payload"), &opts)
            .unwrap();

        assert!(first.borrow().is_empty(), "untagged behaviour untouched");
        assert_eq!(second.borrow().as_slice(), ["payload"]);
    }

    /// A single-behaviour scaffold still delivers a frame whose `opts` carry
    /// no `tag` key at all — the fallback that keeps `<ui-space-name>`,
    /// `<ui-member-roster>`, and `<ui-space-switcher>` alive if the host ever
    /// stops echoing a tag.
    #[dialog_common::test]
    fn it_falls_back_to_the_sole_behaviour_when_a_frame_carries_no_tag() {
        let document = window().unwrap().document().unwrap();
        let host: HtmlElement = document.create_element("div").unwrap().dyn_into().unwrap();
        host.set_attribute("space", "did:key:z6Mk").unwrap();

        let seen = Rc::new(RefCell::new(Vec::new()));
        let scaffold = Scaffold::default();
        scaffold.connect_all(
            &host,
            vec![Rc::new(Recorder {
                tag: "only",
                seen: Rc::clone(&seen),
            })],
        );

        // No `tag` key at all on `opts` — not even an empty string.
        let opts = js_sys::Object::new();
        let reset = Reflect::get(&host, &"__tonkReset".into())
            .unwrap()
            .dyn_into::<Function>()
            .unwrap();
        reset
            .call2(&JsValue::NULL, &JsValue::from_str("payload"), &opts)
            .unwrap();

        assert_eq!(seen.borrow().as_slice(), ["payload"]);
    }

    /// A multi-behaviour scaffold drops a frame carrying a tag that matches
    /// none of its behaviours — a frame addressed to nobody must not be
    /// misdelivered to an arbitrary behaviour.
    #[dialog_common::test]
    fn it_drops_a_frame_whose_tag_matches_no_behaviour() {
        let document = window().unwrap().document().unwrap();
        let host: HtmlElement = document.create_element("div").unwrap().dyn_into().unwrap();
        host.set_attribute("space", "did:key:z6Mk").unwrap();

        let first = Rc::new(RefCell::new(Vec::new()));
        let second = Rc::new(RefCell::new(Vec::new()));
        let scaffold = Scaffold::default();
        scaffold.connect_all(
            &host,
            vec![
                Rc::new(Recorder {
                    tag: "one",
                    seen: Rc::clone(&first),
                }),
                Rc::new(Recorder {
                    tag: "two",
                    seen: Rc::clone(&second),
                }),
            ],
        );

        let opts = js_sys::Object::new();
        Reflect::set(&opts, &"tag".into(), &"unrecognised".into()).unwrap();
        let reset = Reflect::get(&host, &"__tonkReset".into())
            .unwrap()
            .dyn_into::<Function>()
            .unwrap();
        reset
            .call2(&JsValue::NULL, &JsValue::from_str("payload"), &opts)
            .unwrap();

        assert!(first.borrow().is_empty(), "not delivered to \"one\"");
        assert!(second.borrow().is_empty(), "not delivered to \"two\"");
    }

    /// A newly joined space boots its sealed guest while the host is still
    /// establishing subscriptions. The host can claim `tonk-subscribe`
    /// before it has a handle to return; `<tonk-share>` must retry that
    /// incomplete handshake or it never receives the minted invite URL and
    /// the clipboard waits until the 15-second timeout.
    #[dialog_common::test]
    async fn it_retries_an_incomplete_subscription_handshake() {
        let document = window().expect("window").document().expect("document");
        let host: HtmlElement = document.create_element("div").unwrap().dyn_into().unwrap();
        host.set_attribute("space", "did:key:z6MkJoined").unwrap();
        document.body().unwrap().append_child(&host).unwrap();

        let attempts = Rc::new(Cell::new(0_u32));
        let attempts_for_listener = attempts.clone();
        let listener = Closure::<dyn FnMut(CustomEvent)>::new(move |event: CustomEvent| {
            event.prevent_default();
            event.stop_propagation();
            let attempt = attempts_for_listener.get() + 1;
            attempts_for_listener.set(attempt);
            if attempt == 1 {
                return;
            }

            let detail = event.detail();
            let subscription = js_sys::Object::new();
            let cancel = js_sys::Function::new_no_args("");
            Reflect::set(&subscription, &"cancel".into(), cancel.as_ref()).unwrap();
            Reflect::set(&detail, &"subscription".into(), subscription.as_ref()).unwrap();
        });
        host.add_event_listener_with_callback(
            tonk_host::events::SUBSCRIBE,
            listener.as_ref().unchecked_ref(),
        )
        .unwrap();

        let scaffold = Scaffold::default();
        scaffold.connect(
            &host,
            Rc::new(Recorder {
                tag: "joined-share",
                seen: Rc::new(RefCell::new(Vec::new())),
            }),
        );

        wait_ms(750).await;
        assert_eq!(
            attempts.get(),
            2,
            "the incomplete first handshake must be retried",
        );
        assert_eq!(
            scaffold.subscriptions.borrow().len(),
            1,
            "the retry must retain the live subscription handle",
        );

        host.remove();
    }
}
