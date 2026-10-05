//! Hearing `inspect`.
//!
//! The palette's `inspect` command cannot open anything itself: its
//! handler runs in the service worker, which has no page. So it does
//! what the FAB's own acts do and records the request on the asking
//! tab's site — `xyz.tonk.site/request` and `xyz.tonk.site/request-time`
//! on `site:<client>`, in the profile's session overlay (see the
//! worker's `site_request`). Whatever is meant to act subscribes to that
//! site and acts on a request newer than any it has seen.
//!
//! Only the frame whose site the command named hears it — in a space,
//! that is the bar's frame. A space nested inside it registers a site
//! of its own and never sees the request. So this module is only how
//! the *top* of the tab learns of `inspect`; the overlay then relays a
//! pick down the frame tree, and the frame holding the content hears it
//! that way (see `relay_pick` in the overlay).
//!
//! The first frame of the subscription only primes what has been seen.
//! A request made before this overlay connected is not replayed:
//! reloading a page you once inspected should not start inspecting it.

/// The request this module answers. The bar ignores it.
pub const INSPECT: &str = "inspect";

/// What the overlay has heard of the tab's site requests.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Heard {
    /// The newest request time seen. `None` until the first frame.
    seen: Option<f64>,
}

impl Heard {
    /// Consider the newest request on the site. Returns whether it is an
    /// `inspect` newer than anything seen before.
    pub fn consider(&mut self, newest: Option<(&str, f64)>) -> bool {
        let Some(seen) = self.seen else {
            // Prime, never fire, on the first frame.
            self.seen = Some(newest.map_or(0.0, |(_, time)| time));
            return false;
        };
        let Some((request, time)) = newest else {
            return false;
        };
        if time <= seen {
            return false;
        }
        // Any newer request moves the mark, whoever it was for, so an
        // `inspect` that arrived before a `share` is not re-heard after.
        self.seen = Some(time);
        request == INSPECT
    }
}

#[cfg(target_arch = "wasm32")]
pub use dom::{Listening, install_shims, listen};

#[cfg(target_arch = "wasm32")]
mod dom {
    use std::cell::RefCell;
    use std::rc::Rc;

    use js_sys::{Function, Reflect};
    use tonk_host::consumer::{self as host_consumer, Subscription};
    use wasm_bindgen::JsValue;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen_futures::spawn_local;
    use web_sys::{Element, window};

    use super::Heard;

    /// A live subscription to the tab's site requests. Dropping it
    /// cancels the subscription.
    pub struct Listening {
        _subscription: Subscription,
    }

    /// Give `<tonk-introspect>` the `reset` / `update` methods the host
    /// calls with subscription frames, forwarding to per-instance
    /// closures — the same shim `<tonk-display>` and the bar's
    /// subscribers use, since a `Closure` has no `this`.
    pub fn install_shims(tag: &str) {
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
        for (method, slot) in [("reset", "__tonkReset"), ("update", "__tonkUpdate")] {
            let shim = Function::new_with_args(
                "payload, opts",
                &format!("if (typeof this.{slot} === 'function') this.{slot}(payload, opts);"),
            );
            let _ = Reflect::set(&proto, &method.into(), &shim);
        }
    }

    /// Subscribe `host` to the tab's site requests and call `on_inspect`
    /// for each new `inspect`. The subscription lands in `slot` once it
    /// is up; until then, and if it never comes up, nothing is heard.
    pub fn listen(host: &Element, slot: Rc<RefCell<Option<Listening>>>, on_inspect: Rc<dyn Fn()>) {
        let heard = Rc::new(RefCell::new(Heard::default()));
        for (slot_name, rows_of) in [
            (
                "__tonkReset",
                rows_of_reset as fn(&JsValue) -> js_sys::Array,
            ),
            ("__tonkUpdate", rows_of_update),
        ] {
            let heard = heard.clone();
            let on_inspect = on_inspect.clone();
            let closure = Closure::wrap(Box::new(move |payload: JsValue, _opts: JsValue| {
                let rows = rows_of(&payload);
                let newest = read(&rows.get(rows.length().saturating_sub(1)));
                let fire = heard.borrow_mut().consider(
                    newest
                        .as_ref()
                        .map(|(request, time)| (request.as_str(), *time)),
                );
                if fire {
                    on_inspect();
                }
            }) as Box<dyn FnMut(JsValue, JsValue)>);
            let _ = Reflect::set(host, &slot_name.into(), closure.as_ref());
            // Owned by the element for its lifetime, like `draw`.
            closure.forget();
        }

        let host = host.clone();
        spawn_local(async move {
            // The tab's site is `site:<client>`, which the worker derives
            // from the tab that ran the command. A frame that has not
            // registered one asks with `/`, which names the site without
            // claiming a route, as the bar's own listener does.
            let mut site = tonk_host::bridge::site_id();
            if site.is_empty() {
                match tonk_host::bridge::ensure_site("/").await {
                    Ok(assigned) => site = assigned,
                    Err(error) => {
                        warn(&format!(
                            "tonk-introspect: no site for this tab: {}",
                            error.message
                        ));
                        return;
                    }
                }
            }
            if site.is_empty() || !host.is_connected() {
                return;
            }
            let branch = tonk_host::bridge::resolve_profile_branch().await;
            let body = serde_json::json!({
                "predicate": { "with": {
                    "request": { "the": "xyz.tonk.site/request", "as": "Text", "cardinality": "one" },
                    "time": { "the": "xyz.tonk.site/request-time", "as": "Float", "cardinality": "one" }
                } },
                "terms": {
                    "this": site,
                    "request": { "?": { "name": "request" } },
                    "time": { "?": { "name": "time" } }
                }
            });
            let Ok(body) = serde_wasm_bindgen::to_value(&body) else {
                return;
            };
            match host_consumer::subscribe_claimed_with_route(
                &host,
                &body,
                None,
                None,
                Some(&branch),
                true,
            )
            .await
            {
                Ok(subscription) => {
                    *slot.borrow_mut() = Some(Listening {
                        _subscription: subscription,
                    });
                }
                Err(error) => {
                    warn(&format!(
                        "tonk-introspect: cannot hear `inspect`: {}",
                        error.message
                    ));
                }
            }
        });
    }

    fn rows_of_reset(payload: &JsValue) -> js_sys::Array {
        js_sys::Array::from(payload)
    }

    fn rows_of_update(payload: &JsValue) -> js_sys::Array {
        let asserted = Reflect::get(payload, &"asserted".into()).unwrap_or(JsValue::UNDEFINED);
        js_sys::Array::from(&asserted)
    }

    /// `{fields: {request, time}}` off a subscription row.
    fn read(row: &JsValue) -> Option<(String, f64)> {
        if row.is_undefined() || row.is_null() {
            return None;
        }
        let fields = Reflect::get(row, &"fields".into()).ok()?;
        let request = Reflect::get(&fields, &"request".into()).ok()?.as_string()?;
        let time = Reflect::get(&fields, &"time".into()).ok()?.as_f64()?;
        Some((request, time))
    }

    fn warn(message: &str) {
        web_sys::console::warn_1(&JsValue::from_str(message));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_frame_primes_and_never_fires() {
        let mut heard = Heard::default();
        assert!(
            !heard.consider(Some((INSPECT, 10.0))),
            "reloading a page once inspected must not start inspecting it"
        );
    }

    #[test]
    fn a_newer_inspect_fires() {
        let mut heard = Heard::default();
        heard.consider(Some((INSPECT, 10.0)));
        assert!(heard.consider(Some((INSPECT, 11.0))));
    }

    #[test]
    fn the_same_request_seen_again_does_not_fire_twice() {
        let mut heard = Heard::default();
        heard.consider(None);
        assert!(heard.consider(Some((INSPECT, 5.0))));
        assert!(
            !heard.consider(Some((INSPECT, 5.0))),
            "a re-delivered frame"
        );
    }

    #[test]
    fn another_request_is_the_bars_and_does_not_fire() {
        let mut heard = Heard::default();
        heard.consider(None);
        assert!(!heard.consider(Some(("share", 5.0))));
    }

    #[test]
    fn a_request_for_the_bar_still_moves_the_mark() {
        let mut heard = Heard::default();
        heard.consider(None);
        heard.consider(Some((INSPECT, 5.0)));
        heard.consider(Some(("share", 6.0)));
        assert!(
            !heard.consider(Some((INSPECT, 5.0))),
            "an older inspect is not re-heard after a newer share"
        );
    }

    #[test]
    fn an_empty_site_primes_at_zero() {
        let mut heard = Heard::default();
        heard.consider(None);
        assert!(heard.consider(Some((INSPECT, 0.5))));
    }
}
