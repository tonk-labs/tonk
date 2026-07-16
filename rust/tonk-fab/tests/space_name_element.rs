//! `<ui-space-name>` in a real DOM.
//!
//! The FAB reads a space's name from that space's own branch without any
//! seeded view. That rests on three behaviours no native test can reach: the
//! element registers, it stamps its OWN routing context (`resolve_with` never
//! walks ancestors), and it dispatches a subscribe carrying a RAW ATTRIBUTE
//! query — naming a concept would reintroduce the frozen-descriptor
//! dependency the whole design removes.
//!
//! No host is installed here, so nothing answers the event and no frame
//! arrives. That is deliberate: this pins what the ELEMENT does. Host
//! delivery is proven in production by `<ui-sync-status>`.
//!
//! Also covers `<ui-member-roster>`, built on the same `subscribing`
//! scaffolding as `<ui-space-name>` — see the tests below the space-name
//! ones. Both elements are proven against the identical `reset`/`update`
//! delegate contract the scaffolding installs.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::wasm_bindgen_test_configure;
use web_sys::{CustomEvent, window};

wasm_bindgen_test_configure!(run_in_browser);

const SPACE: &str = "did:key:z6MkTestSpace";

fn document() -> web_sys::Document {
    window().expect("window").document().expect("document")
}

/// Yield to the event loop for `ms` milliseconds using the native
/// `setTimeout`/`Promise` bridge already available through this crate's
/// `web-sys`/`js-sys`/`wasm-bindgen-futures` dependencies — no extra
/// third-party crate needed just to await a tick.
async fn yield_for(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let win = window().expect("window");
        win.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .expect("set_timeout");
    });
    wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .expect("timeout resolves");
}

/// Mount a `<ui-space-name space=SPACE>` and return it.
fn mount() -> web_sys::HtmlElement {
    tonk_fab::register();
    let el = document()
        .create_element("ui-space-name")
        .expect("create")
        .dyn_into::<web_sys::HtmlElement>()
        .expect("html element");
    el.set_attribute("space", SPACE).expect("set space");
    document()
        .body()
        .expect("body")
        .append_child(el.as_ref())
        .expect("append");
    el
}

#[dialog_common::test]
async fn it_registers_as_a_custom_element() {
    tonk_fab::register();
    let defined = window()
        .expect("window")
        .custom_elements()
        .get("ui-space-name");
    assert!(
        !defined.is_undefined(),
        "tonk_fab::register() must define <ui-space-name>"
    );
}

#[dialog_common::test]
async fn it_stamps_its_own_routing_context_from_the_space_attribute() {
    let el = mount();
    // `resolve_with` reads THIS element's own `with` and never walks
    // ancestors, so the element must stamp it itself — unlike
    // <ui-sync-status>, which receives `with` from a view template.
    assert_eq!(
        el.get_attribute("with").as_deref(),
        Some("main@did:key:z6MkTestSpace"),
        "element must stamp its own with= from `space`"
    );
}

#[dialog_common::test]
async fn it_dispatches_a_subscribe_carrying_the_raw_attribute_query() {
    // Capture `tonk-subscribe` before mounting; it bubbles and is composed,
    // so a document-level listener sees it.
    let seen: std::rc::Rc<std::cell::RefCell<Option<String>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let sink = seen.clone();
    let cb = Closure::<dyn FnMut(CustomEvent)>::new(move |ev: CustomEvent| {
        let detail = ev.detail();
        let json = js_sys::JSON::stringify(&detail)
            .map(|s| String::from(s))
            .unwrap_or_default();
        *sink.borrow_mut() = Some(json);
    });
    document()
        .add_event_listener_with_callback("tonk-subscribe", cb.as_ref().unchecked_ref())
        .expect("listen");

    mount();

    // The element subscribes from a spawn_local on connect; yield to let it run.
    yield_for(50).await;

    let captured = seen.borrow().clone();
    let detail = captured.expect("element must dispatch tonk-subscribe on connect");

    // The RAW attribute URI — nothing seeded is consulted.
    assert!(
        detail.contains("xyz.tonk.repo/name"),
        "subscribe must query the raw attribute: {detail}"
    );
    // Naming a concept would reintroduce the frozen-descriptor dependency.
    assert!(
        !detail.contains("tonk:repository"),
        "subscribe must NOT name a concept: {detail}"
    );
    // Bound to this space's subject.
    assert!(
        detail.contains(SPACE),
        "subscribe must bind the subject: {detail}"
    );

    drop(cb);
}

#[dialog_common::test]
async fn it_dispatches_an_inlined_rename_claim_and_reverts_the_chip_immediately() {
    // Mock `window.tonk.transact` to capture the routeless dispatch — there is
    // no installed host in this test (see the module doc), so nothing else
    // would observe it.
    let captured: std::rc::Rc<std::cell::RefCell<Option<String>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let sink = captured.clone();
    let transact_cb = Closure::<dyn FnMut(JsValue)>::new(move |req: JsValue| {
        let json = js_sys::JSON::stringify(&req)
            .map(String::from)
            .unwrap_or_default();
        *sink.borrow_mut() = Some(json);
    });
    let win = window().expect("window");
    let tonk = js_sys::Object::new();
    js_sys::Reflect::set(&tonk, &"transact".into(), transact_cb.as_ref()).expect("set transact");
    js_sys::Reflect::set(&win, &"tonk".into(), &tonk).expect("set window.tonk");

    let el = mount();
    let editable = el
        .query_selector("tonk-editable")
        .expect("query")
        .expect("<tonk-editable> child rendered");
    assert_eq!(
        editable.text_content().as_deref(),
        Some("Untitled"),
        "chip renders the placeholder before any frame arrives"
    );

    // Simulate an edit commit: `<tonk-editable>` sets its own text then
    // dispatches a bubbling `change` on blur — mirror that directly rather
    // than depending on its dblclick/focus/blur choreography, which isn't
    // registered here (no host, no `tonk-workspace::register()` in this crate).
    editable.set_text_content(Some("Renamed Space"));
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    let change = web_sys::Event::new_with_event_init_dict("change", &init).expect("event");
    editable.dispatch_event(&change).expect("dispatch change");

    yield_for(10).await;

    let json = captured
        .borrow()
        .clone()
        .expect("window.tonk.transact must be called on commit");
    // The descriptor rides WITH the claim — nothing seeded is consulted.
    assert!(
        json.contains("xyz.tonk.rename-repository/space"),
        "claim must inline the rename-repository descriptor: {json}"
    );
    assert!(
        json.contains(SPACE),
        "claim must name the target space: {json}"
    );
    assert!(
        json.contains("Renamed Space"),
        "claim must carry the typed name: {json}"
    );

    // The chip never optimistically keeps the typed text: by the time the
    // claim dispatches it must already show the prior (pre-frame) name again
    // — nothing here confirms the rename actually committed.
    assert_eq!(
        editable.text_content().as_deref(),
        Some("Untitled"),
        "chip must revert to the previous name, not keep the typed one"
    );

    js_sys::Reflect::delete_property(&win, &"tonk".into()).ok();
    drop(transact_cb);
}

/// A single conclusion row shaped like a real subscription result:
/// `{ fields: { name } }`.
fn conclusion_row(name: &str) -> JsValue {
    let fields = js_sys::Object::new();
    js_sys::Reflect::set(&fields, &"name".into(), &JsValue::from_str(name)).expect("set name");
    let row = js_sys::Object::new();
    js_sys::Reflect::set(&row, &"fields".into(), &fields).expect("set fields");
    row.into()
}

/// A `reset` snapshot payload: a bare array of conclusion rows, as
/// `tonk-host::consumer` delivers on the first (or reconnect) frame.
fn reset_payload(name: &str) -> JsValue {
    let rows = js_sys::Array::new();
    rows.push(&conclusion_row(name));
    rows.into()
}

/// An `update` delta payload: `{ asserted, retracted }`.
fn update_payload(name: &str) -> JsValue {
    let asserted = js_sys::Array::new();
    asserted.push(&conclusion_row(name));
    let payload = js_sys::Object::new();
    js_sys::Reflect::set(&payload, &"asserted".into(), &asserted).expect("set asserted");
    payload.into()
}

/// Invoke `element.reset`/`element.update` exactly as the host's
/// `deliver_frame` does: read the method off the element (installed on the
/// prototype by `install_frame_shims`) and call it with `element` as `this`.
fn deliver(el: &web_sys::HtmlElement, method: &str, payload: &JsValue) {
    let opts = js_sys::Object::new();
    js_sys::Reflect::set(&opts, &"tag".into(), &JsValue::from_str("ui-space-name"))
        .expect("set tag");
    let f = js_sys::Reflect::get(el, &method.into())
        .unwrap_or_else(|_| panic!("{method} present on element"))
        .dyn_into::<js_sys::Function>()
        .unwrap_or_else(|_| panic!("{method} is a function"));
    f.call2(el, payload, &opts.into())
        .unwrap_or_else(|_| panic!("{method} call"));
}

#[dialog_common::test]
async fn it_renders_the_name_from_a_delivered_frame() {
    // No host is installed in this crate's tests (see the module doc), so
    // deliver frames the exact way `tonk-host::ops::deliver_frame` does:
    // call the `reset`/`update` methods directly on the element. This is the
    // path that shipped broken — the element subscribed and asked the right
    // question, but had no delegate wired up to consume the answer, so the
    // chip stayed on "Untitled" forever. See commit 71d1c58ac.
    let el = mount();
    let editable = el
        .query_selector("tonk-editable")
        .expect("query")
        .expect("<tonk-editable> child rendered");
    assert_eq!(
        editable.text_content().as_deref(),
        Some("Untitled"),
        "chip renders the placeholder before any frame arrives"
    );

    deliver(&el, "reset", &reset_payload("Real Spot"));

    assert_eq!(
        editable.text_content().as_deref(),
        Some("Real Spot"),
        "a delivered reset frame must be consumed and rendered, not ignored"
    );

    // A subsequent `update` delta must also be consumed and re-render the chip.
    deliver(&el, "update", &update_payload("Renamed Again"));

    assert_eq!(
        editable.text_content().as_deref(),
        Some("Renamed Again"),
        "a delivered update frame must be consumed and rendered, not ignored"
    );
}

// --- <ui-member-roster>, built on the same subscribing scaffolding ---
//
// These mirror the `<ui-space-name>` tests above rather than living in a
// separate file: both elements are proven against the same `reset`/`update`
// delegate contract the scaffolding installs, so keeping them side by side
// makes that shared contract visible in one place. Nothing above is edited.

const ROSTER_TAG: &str = "ui-member-roster";

/// Mount a `<ui-member-roster space=SPACE>` and return it.
fn mount_roster() -> web_sys::HtmlElement {
    tonk_fab::register();
    let el = document()
        .create_element(ROSTER_TAG)
        .expect("create")
        .dyn_into::<web_sys::HtmlElement>()
        .expect("html element");
    el.set_attribute("space", SPACE).expect("set space");
    document()
        .body()
        .expect("body")
        .append_child(el.as_ref())
        .expect("append");
    el
}

/// A single member conclusion row, shaped like a real subscription result:
/// `{ this, fields: { this, member, role, name } }`.
fn member_row(this_id: &str, name: &str) -> JsValue {
    let fields = js_sys::Object::new();
    js_sys::Reflect::set(&fields, &"this".into(), &JsValue::from_str(this_id)).expect("set this");
    js_sys::Reflect::set(
        &fields,
        &"member".into(),
        &JsValue::from_str("did:key:zMember"),
    )
    .expect("set member");
    js_sys::Reflect::set(&fields, &"role".into(), &JsValue::from_str("tonk:member"))
        .expect("set role");
    js_sys::Reflect::set(&fields, &"name".into(), &JsValue::from_str(name)).expect("set name");
    let row = js_sys::Object::new();
    js_sys::Reflect::set(&row, &"this".into(), &JsValue::from_str(this_id)).expect("set row this");
    js_sys::Reflect::set(&row, &"fields".into(), &fields).expect("set fields");
    row.into()
}

/// A `reset` snapshot payload: a bare array of member rows.
fn roster_reset_payload(rows: &[(&str, &str)]) -> JsValue {
    let arr = js_sys::Array::new();
    for (id, name) in rows {
        arr.push(&member_row(id, name));
    }
    arr.into()
}

/// An `update` delta payload: `{ asserted, retracted }`. Retracted rows only
/// need to carry `this` to identify what to remove.
fn roster_update_payload(asserted: &[(&str, &str)], retracted: &[&str]) -> JsValue {
    let asserted_arr = js_sys::Array::new();
    for (id, name) in asserted {
        asserted_arr.push(&member_row(id, name));
    }
    let retracted_arr = js_sys::Array::new();
    for id in retracted {
        retracted_arr.push(&member_row(id, ""));
    }
    let payload = js_sys::Object::new();
    js_sys::Reflect::set(&payload, &"asserted".into(), &asserted_arr).expect("set asserted");
    js_sys::Reflect::set(&payload, &"retracted".into(), &retracted_arr).expect("set retracted");
    payload.into()
}

/// Invoke `element.reset`/`element.update` for the roster tag, exactly as the
/// host's `deliver_frame` does.
fn deliver_roster(el: &web_sys::HtmlElement, method: &str, payload: &JsValue) {
    let opts = js_sys::Object::new();
    js_sys::Reflect::set(&opts, &"tag".into(), &JsValue::from_str(ROSTER_TAG)).expect("set tag");
    let f = js_sys::Reflect::get(el, &method.into())
        .unwrap_or_else(|_| panic!("{method} present on element"))
        .dyn_into::<js_sys::Function>()
        .unwrap_or_else(|_| panic!("{method} is a function"));
    f.call2(el, payload, &opts.into())
        .unwrap_or_else(|_| panic!("{method} call"));
}

/// Read the rendered member names off the roster host, in DOM order.
fn rendered_names(el: &web_sys::HtmlElement) -> Vec<String> {
    let children = el.children();
    (0..children.length())
        .filter_map(|i| children.item(i))
        .filter(|c| c.class_list().contains("fab__menu-item--member"))
        .filter_map(|c| c.text_content())
        .collect()
}

#[dialog_common::test]
async fn it_registers_the_roster_as_a_custom_element() {
    tonk_fab::register();
    let defined = window().expect("window").custom_elements().get(ROSTER_TAG);
    assert!(
        !defined.is_undefined(),
        "tonk_fab::register() must define <ui-member-roster>"
    );
}

#[dialog_common::test]
async fn it_renders_the_roster_from_delivered_frames() {
    // Mirrors `it_renders_the_name_from_a_delivered_frame`: no host is
    // installed in this crate's tests, so deliver frames the exact way
    // `tonk-host::ops::deliver_frame` does — call `reset`/`update` directly
    // on the element. An element that subscribes and never renders is the
    // bug this whole scaffolding exists to catch.
    let el = mount_roster();
    assert!(
        rendered_names(&el).is_empty(),
        "no members render before any frame arrives"
    );

    deliver_roster(
        &el,
        "reset",
        &roster_reset_payload(&[("member:1", "Alice"), ("member:2", "Bob")]),
    );
    assert_eq!(
        rendered_names(&el),
        vec!["Alice".to_string(), "Bob".to_string()],
        "a delivered reset frame must be consumed and rendered as one span per member"
    );

    // A subsequent `update` delta must also be consumed: retract Alice,
    // assert Carol — Bob, untouched by the delta, must remain.
    deliver_roster(
        &el,
        "update",
        &roster_update_payload(&[("member:3", "Carol")], &["member:1"]),
    );
    assert_eq!(
        rendered_names(&el),
        vec!["Bob".to_string(), "Carol".to_string()],
        "a delivered update frame must retract, assert, and re-render, not be ignored"
    );
}

// --- <ui-space-switcher>, built on the same subscribing scaffolding ---
//
// Mirrors the `<ui-member-roster>` tests above: no host is installed in this
// crate's tests (see the module doc), so frames are delivered the exact way
// `tonk-host::ops::deliver_frame` does — call `reset`/`update` directly on
// the element. An element that subscribes and never renders is the bug this
// whole scaffolding exists to catch — see `it_renders_the_roster_from_delivered_frames`
// above and commit 71d1c58ac.
//
// The switcher is also the one element whose routing context is NOT derived
// from a `space` attribute: it always reads the PROFILE branch
// (`main@profile:tonk`), proving the `Subscribing::resolve_with` seam
// accepts a routing context that isn't space-derived.

const SWITCHER_TAG: &str = "ui-space-switcher";
const ACTIVE_SPACE: &str = "did:key:z6MkActiveSpace";
const OTHER_SPACE: &str = "did:key:z6MkOtherSpace";
const THIRD_SPACE: &str = "did:key:z6MkThirdSpace";

/// Mount a `<ui-space-switcher exclude=ACTIVE_SPACE>` and return it.
fn mount_switcher() -> web_sys::HtmlElement {
    tonk_fab::register();
    let el = document()
        .create_element(SWITCHER_TAG)
        .expect("create")
        .dyn_into::<web_sys::HtmlElement>()
        .expect("html element");
    el.set_attribute("exclude", ACTIVE_SPACE)
        .expect("set exclude");
    document()
        .body()
        .expect("body")
        .append_child(el.as_ref())
        .expect("append");
    el
}

/// A single replica conclusion row, shaped like a real subscription result:
/// `{ this, fields: { subject, kind, status } }`.
fn replica_row(this_id: &str, subject: &str, kind: &str, status: &str) -> JsValue {
    let fields = js_sys::Object::new();
    js_sys::Reflect::set(&fields, &"subject".into(), &JsValue::from_str(subject))
        .expect("set subject");
    js_sys::Reflect::set(&fields, &"kind".into(), &JsValue::from_str(kind)).expect("set kind");
    js_sys::Reflect::set(&fields, &"status".into(), &JsValue::from_str(status))
        .expect("set status");
    let row = js_sys::Object::new();
    js_sys::Reflect::set(&row, &"this".into(), &JsValue::from_str(this_id)).expect("set row this");
    js_sys::Reflect::set(&row, &"fields".into(), &fields).expect("set fields");
    row.into()
}

/// A `reset` snapshot payload: a bare array of replica rows.
fn switcher_reset_payload(rows: &[(&str, &str, &str, &str)]) -> JsValue {
    let arr = js_sys::Array::new();
    for (id, subject, kind, status) in rows {
        arr.push(&replica_row(id, subject, kind, status));
    }
    arr.into()
}

/// An `update` delta payload: `{ asserted, retracted }`. Retracted rows only
/// need to carry `this` to identify what to remove.
fn switcher_update_payload(asserted: &[(&str, &str, &str, &str)], retracted: &[&str]) -> JsValue {
    let asserted_arr = js_sys::Array::new();
    for (id, subject, kind, status) in asserted {
        asserted_arr.push(&replica_row(id, subject, kind, status));
    }
    let retracted_arr = js_sys::Array::new();
    for id in retracted {
        retracted_arr.push(&replica_row(id, "", "", ""));
    }
    let payload = js_sys::Object::new();
    js_sys::Reflect::set(&payload, &"asserted".into(), &asserted_arr).expect("set asserted");
    js_sys::Reflect::set(&payload, &"retracted".into(), &retracted_arr).expect("set retracted");
    payload.into()
}

/// Invoke `element.reset`/`element.update` for the switcher tag, exactly as
/// the host's `deliver_frame` does.
fn deliver_switcher(el: &web_sys::HtmlElement, method: &str, payload: &JsValue) {
    let opts = js_sys::Object::new();
    js_sys::Reflect::set(&opts, &"tag".into(), &JsValue::from_str(SWITCHER_TAG)).expect("set tag");
    let f = js_sys::Reflect::get(el, &method.into())
        .unwrap_or_else(|_| panic!("{method} present on element"))
        .dyn_into::<js_sys::Function>()
        .unwrap_or_else(|_| panic!("{method} is a function"));
    f.call2(el, payload, &opts.into())
        .unwrap_or_else(|_| panic!("{method} call"));
}

/// Read the rendered row anchors' `href` off the switcher host, in DOM order
/// — `.fab__menu-item` WITHOUT `--action`, i.e. the space rows, not the
/// static "all spots"/"new" items.
fn rendered_row_hrefs(el: &web_sys::HtmlElement) -> Vec<String> {
    let children = el.children();
    (0..children.length())
        .filter_map(|i| children.item(i))
        .filter(|c| {
            c.class_list().contains("fab__menu-item")
                && !c.class_list().contains("fab__menu-item--action")
        })
        .filter_map(|c| c.get_attribute("href"))
        .collect()
}

#[dialog_common::test]
async fn it_registers_the_switcher_as_a_custom_element() {
    tonk_fab::register();
    let defined = window()
        .expect("window")
        .custom_elements()
        .get(SWITCHER_TAG);
    assert!(
        !defined.is_undefined(),
        "tonk_fab::register() must define <ui-space-switcher>"
    );
}

#[dialog_common::test]
async fn it_stamps_the_profile_routing_context_not_a_space_derived_one() {
    let el = mount_switcher();
    assert_eq!(
        el.get_attribute("with").as_deref(),
        Some("main@profile:tonk"),
        "the switcher must stamp the fixed profile routing context, not a \
         space-derived one"
    );
}

#[dialog_common::test]
async fn it_renders_rows_from_delivered_frames_filtering_self_and_the_active_space() {
    // No host is installed in this crate's tests, so deliver frames the
    // exact way `tonk-host::ops::deliver_frame` does. This is the path that
    // shipped broken for <ui-space-name>: an element that subscribes and
    // asks the right question but never consumes the answer.
    let el = mount_switcher();
    assert!(
        rendered_row_hrefs(&el).is_empty(),
        "no rows render before any frame arrives"
    );

    deliver_switcher(
        &el,
        "reset",
        &switcher_reset_payload(&[
            // The profile's own self-replica: must be filtered, kind == tonk:profile.
            (
                "replica:profile",
                "did:key:z6MkProfileSelf",
                "tonk:profile",
                "tonk:active",
            ),
            // The active space (this element's `exclude`): must be filtered.
            (
                "replica:active",
                ACTIVE_SPACE,
                "tonk:repository",
                "tonk:active",
            ),
            // A genuine other space: must render.
            (
                "replica:other",
                OTHER_SPACE,
                "tonk:repository",
                "tonk:active",
            ),
        ]),
    );

    let hrefs = rendered_row_hrefs(&el);
    assert_eq!(
        hrefs,
        vec![format!("/space/{OTHER_SPACE}")],
        "only the non-self, non-active replica renders as a row: {hrefs:?}"
    );

    // The surviving row must wrap a <ui-space-name> for that space's OWN
    // repo name (not the profile-side replica name), and must stamp
    // data-status from the replica's own sync status.
    let row = el
        .query_selector(&format!("a[href=\"/space/{OTHER_SPACE}\"]"))
        .expect("query")
        .expect("surviving row rendered");
    assert_eq!(
        row.get_attribute("data-status").as_deref(),
        Some("tonk:active"),
        "row must stamp data-status from the replica's status"
    );
    let name_el = row
        .query_selector("ui-space-name")
        .expect("query")
        .expect("row must nest a <ui-space-name> for the space's own repo name");
    assert_eq!(
        name_el.get_attribute("space").as_deref(),
        Some(OTHER_SPACE),
        "<ui-space-name> must be scoped to this row's own space, not the profile"
    );

    // A subsequent `update` delta must also be consumed: retract the
    // surviving row, assert a new one — the bug this test exists to catch is
    // an element that subscribes and renders once, then silently ignores
    // every later delta.
    deliver_switcher(
        &el,
        "update",
        &switcher_update_payload(
            &[(
                "replica:third",
                THIRD_SPACE,
                "tonk:repository",
                "tonk:blank",
            )],
            &["replica:other"],
        ),
    );
    let hrefs = rendered_row_hrefs(&el);
    assert_eq!(
        hrefs,
        vec![format!("/space/{THIRD_SPACE}")],
        "an update delta must retract, assert, and re-render, not be ignored: {hrefs:?}"
    );
}

#[dialog_common::test]
async fn it_appends_the_static_action_items_after_the_rows() {
    let el = mount_switcher();
    deliver_switcher(
        &el,
        "reset",
        &switcher_reset_payload(&[(
            "replica:other",
            OTHER_SPACE,
            "tonk:repository",
            "tonk:active",
        )]),
    );

    let children = el.children();
    let tags: Vec<String> = (0..children.length())
        .filter_map(|i| children.item(i))
        .map(|c| c.tag_name().to_lowercase())
        .collect();
    // One row (an <a>), then the "all spots" <a> action, then the "new"
    // <button> action — actions always trail the rows.
    assert_eq!(
        tags,
        vec!["a", "a", "button"],
        "rows must precede the static action items: {tags:?}"
    );

    let all_spots = el
        .query_selector("a.fab__menu-item--action")
        .expect("query")
        .expect("all-spots action item rendered");
    assert_eq!(all_spots.get_attribute("href").as_deref(), Some("/"));
    assert!(
        all_spots
            .text_content()
            .unwrap_or_default()
            .contains("all spots"),
        "all-spots item must read \"all spots\""
    );

    let new_button = el
        .query_selector("button.fab__menu-item--action")
        .expect("query")
        .expect("new action item rendered");
    assert_eq!(new_button.get_attribute("type").as_deref(), Some("button"));
    assert_eq!(
        new_button.get_attribute("data-dialog").as_deref(),
        Some("open fab-space-create")
    );
    assert!(
        new_button
            .text_content()
            .unwrap_or_default()
            .contains("new"),
        "new item must read \"new\""
    );
}
