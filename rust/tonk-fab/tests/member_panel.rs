//! The live roster remains attached to the FABB and handles frame deltas.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use js_sys::{Array, Function, Object, Promise, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::wasm_bindgen_test_configure;
use web_sys::{CustomEvent, CustomEventInit, Element, HtmlElement, window};

wasm_bindgen_test_configure!(run_in_browser);

fn mount() -> (HtmlElement, HtmlElement) {
    tonk_fab::register();
    let document = window().unwrap().document().unwrap();
    let bar: HtmlElement = document
        .create_element("tonk-fab")
        .unwrap()
        .unchecked_into();
    bar.set_attribute("space", "did:key:zMembers").unwrap();
    document.body().unwrap().append_child(&bar).unwrap();
    let roster = bar
        .query_selector("ui-member-roster")
        .unwrap()
        .unwrap()
        .unchecked_into();
    (bar, roster)
}

fn shadow(bar: &HtmlElement, selector: &str) -> Element {
    bar.shadow_root()
        .unwrap()
        .query_selector(selector)
        .unwrap()
        .unwrap()
}

fn deliver(roster: &HtmlElement, method: &str, payload: serde_json::Value) {
    deliver_tag(roster, method, payload, "ui-member-roster");
}

fn deliver_tag(roster: &HtmlElement, method: &str, payload: serde_json::Value, tag: &str) {
    let payload = js_sys::JSON::parse(&payload.to_string()).unwrap();
    let opts = Object::new();
    Reflect::set(&opts, &"tag".into(), &tag.into()).unwrap();
    Reflect::get(roster, &method.into())
        .unwrap()
        .dyn_into::<Function>()
        .unwrap()
        .call2(roster, &payload, &opts)
        .unwrap();
}

fn member(id: &str, name: &str) -> serde_json::Value {
    serde_json::json!({
        "this": id,
        "fields": { "name": name, "member": format!("did:key:{id}"), "role": "tonk:member" }
    })
}

async fn settle() {
    let promise = Promise::new(&mut |resolve, _| {
        window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 20)
            .unwrap();
    });
    JsFuture::from(promise).await.unwrap();
}

#[dialog_common::test]
async fn repository_self_member_gets_a_separate_you_marker_and_refreshes_after_account_change() {
    let win = window().unwrap();
    let original_fetch = Reflect::get(&win, &"fetch".into()).unwrap();
    let stub = |self_did: &str| {
        Function::new_with_args(
            "url",
            &format!(
                "return Promise.resolve(new Response(JSON.stringify({{members:[{{did:'did:key:owner',is_self:{}}},{{did:'did:key:member',is_self:{}}}]}}),{{status:200}}))",
                self_did == "did:key:owner",
                self_did == "did:key:member"
            ),
        )
    };
    Reflect::set(&win, &"fetch".into(), &stub("did:key:owner")).unwrap();
    let (bar, roster) = mount();
    deliver(
        &roster,
        "reset",
        serde_json::json!([
            { "this": "owner", "fields": { "name": "Owner", "member": "did:key:owner", "role": "tonk:founder" } },
            { "this": "member", "fields": { "name": "Member", "member": "did:key:member", "role": "tonk:member" } }
        ]),
    );
    settle().await;
    let panel = shadow(&bar, ".members-list");
    assert_eq!(
        panel
            .query_selector(".mem-self")
            .unwrap()
            .unwrap()
            .text_content()
            .as_deref(),
        Some("Owner")
    );
    assert_eq!(
        panel
            .query_selector(".mem-you")
            .unwrap()
            .unwrap()
            .text_content()
            .as_deref(),
        Some("you")
    );
    assert_eq!(
        panel
            .query_selector("[data-member=\"did:key:owner\"] .mem-role")
            .unwrap()
            .unwrap()
            .text_content()
            .as_deref(),
        Some("owner")
    );

    Reflect::set(&win, &"fetch".into(), &stub("did:key:member")).unwrap();
    let detail = Object::new();
    Reflect::set(&detail, &"result".into(), &"completed".into()).unwrap();
    let init = CustomEventInit::new();
    init.set_detail(&detail);
    win.dispatch_event(&CustomEvent::new_with_event_init_dict("tonk:task-closed", &init).unwrap())
        .unwrap();
    settle().await;
    assert_eq!(
        panel
            .query_selector(".mem-self")
            .unwrap()
            .unwrap()
            .text_content()
            .as_deref(),
        Some("Member")
    );
    assert_eq!(
        panel
            .query_selector(".mem-you")
            .unwrap()
            .unwrap()
            .text_content()
            .as_deref(),
        Some("you")
    );
    assert_eq!(
        panel
            .query_selector("[data-member=\"did:key:owner\"] .mem-role")
            .unwrap()
            .unwrap()
            .text_content()
            .as_deref(),
        Some("owner")
    );

    bar.remove();
    Reflect::set(&win, &"fetch".into(), &original_fetch).unwrap();
}

#[dialog_common::test]
async fn reset_update_and_retract_render_inside_the_attached_panel() {
    let (bar, roster) = mount();
    assert_eq!(
        shadow(&bar, ".members-list")
            .text_content()
            .unwrap_or_default(),
        "no members are available"
    );

    let ada = member("ada", "Ada");
    let lin = member("lin", "Lin");
    deliver(&roster, "reset", serde_json::json!([ada, lin]));
    shadow(&bar, ".space")
        .unchecked_into::<HtmlElement>()
        .click();
    shadow(&bar, ".members")
        .unchecked_into::<HtmlElement>()
        .click();
    let panel = shadow(&bar, "#members-panel");
    assert!(!panel.has_attribute("hidden"));
    assert_eq!(panel.query_selector_all(".mem-row").unwrap().length(), 2);
    assert_eq!(
        shadow(&bar, ".members span").text_content().as_deref(),
        Some("view members")
    );

    deliver(
        &roster,
        "update",
        serde_json::json!({ "asserted": [], "retracted": [member("ada", "Ada")] }),
    );
    assert_eq!(panel.query_selector_all(".mem-row").unwrap().length(), 1);
    let text = panel.text_content().unwrap_or_default();
    assert!(text.contains("Lin"));
    assert!(!text.contains("Ada"));

    bar.remove();
}

#[dialog_common::test]
async fn twelve_people_remain_readable_without_invented_presence_or_agent_edges() {
    let (bar, roster) = mount();
    let rows = Array::new();
    for index in 0..12 {
        rows.push(
            &js_sys::JSON::parse(
                &member(&format!("m{index}"), &format!("Member {index}")).to_string(),
            )
            .unwrap(),
        );
    }
    let opts = Object::new();
    Reflect::set(&opts, &"tag".into(), &"ui-member-roster".into()).unwrap();
    Reflect::get(&roster, &"reset".into())
        .unwrap()
        .dyn_into::<Function>()
        .unwrap()
        .call2(&roster, &rows, &opts)
        .unwrap();

    let panel = shadow(&bar, ".members-list");
    assert_eq!(panel.query_selector_all(".mem-row").unwrap().length(), 12);
    assert!(
        panel
            .text_content()
            .unwrap_or_default()
            .contains("Member 11")
    );
    assert!(
        panel
            .query_selector("[data-last-active]")
            .unwrap()
            .is_none(),
        "unknown activity is left unknown"
    );
    bar.remove();
}

#[dialog_common::test]
async fn invitation_graph_updates_late_provenance_and_removes_stale_edges() {
    let (bar, roster) = mount();
    let owner = serde_json::json!({"this":"owner", "fields": {"name":"Owner", "member":"did:key:owner", "role":"tonk:founder"}});
    let mut ada = member("ada", "Ada <safe>");
    ada["fields"]["invitation"] = "owner-ada".into();
    let mut lin = member("lin", "Lin");
    lin["fields"]["invitation"] = "ada-lin".into();
    deliver(&roster, "reset", serde_json::json!([owner, ada, lin]));
    let panel = shadow(&bar, ".members-list");
    assert_eq!(panel.query_selector_all("line").unwrap().length(), 1);
    let invitations = serde_json::json!([
        {"this":"owner-ada", "fields":{"inviter":"did:key:owner"}},
        {"this":"ada-lin", "fields":{"inviter":"did:key:ada"}}
    ]);
    deliver_tag(
        &roster,
        "reset",
        invitations.clone(),
        "ui-member-invitations",
    );
    assert_eq!(panel.query_selector_all("line").unwrap().length(), 3);
    for selector in [
        "line[data-from='space'][data-to='did:key:owner']",
        "line[data-from='did:key:owner'][data-to='did:key:ada']",
        "line[data-from='did:key:ada'][data-to='did:key:lin']",
    ] {
        assert!(
            panel.query_selector(selector).unwrap().is_some(),
            "missing {selector}"
        );
    }
    assert!(panel.query_selector("safe").unwrap().is_none());
    assert!(
        panel
            .query_selector("[data-member='did:key:lin']")
            .unwrap()
            .unwrap()
            .get_attribute("aria-label")
            .unwrap()
            .contains("invited by Ada <safe>")
    );
    shadow(&bar, ".space")
        .unchecked_into::<HtmlElement>()
        .click();
    shadow(&bar, ".members")
        .unchecked_into::<HtmlElement>()
        .click();
    settle().await;
    let settled = Promise::new(&mut |resolve, _| {
        window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 500)
            .unwrap();
    });
    JsFuture::from(settled).await.unwrap();
    let centre = shadow(&bar, ".member-space").get_bounding_client_rect();
    let bounds = panel.get_bounding_client_rect();
    assert!((centre.x() + centre.width() / 2.0 - bounds.x() - bounds.width() / 2.0).abs() < 2.0);
    assert!(
        shadow(&bar, "#members-panel")
            .query_selector(".members-help")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        shadow(&bar, ".member-count").text_content().as_deref(),
        Some("3")
    );
    let lin_node = panel
        .query_selector("[data-member='did:key:lin']")
        .unwrap()
        .unwrap()
        .unchecked_into::<HtmlElement>();
    lin_node.click();
    assert_eq!(
        lin_node.get_attribute("aria-pressed").as_deref(),
        Some("true")
    );
    let detail = shadow(&bar, ".member-detail");
    assert!(!detail.has_attribute("hidden"));
    assert!(
        detail
            .text_content()
            .unwrap()
            .contains("invited by Ada <safe>")
    );
    lin_node.click();
    assert!(detail.has_attribute("hidden"));
    let initial_scroll = panel.scroll_top();
    let pan = Function::new_with_args(
        "panel",
        "for (const [type,y] of [['pointerdown',120],['pointermove',90],['pointerup',90]]) panel.dispatchEvent(new PointerEvent(type,{bubbles:true,composed:true,pointerId:7,button:0,clientX:100,clientY:y}));",
    );
    pan.call1(&wasm_bindgen::JsValue::NULL, &panel).unwrap();
    assert!(panel.scroll_top() > initial_scroll, "drag moves the map");
    assert!(!panel.has_attribute("data-panning"));
    let before: f64 = panel.get_attribute("data-scale").unwrap().parse().unwrap();
    shadow(&bar, ".members-zoom-in")
        .unchecked_into::<HtmlElement>()
        .click();
    let after: f64 = panel.get_attribute("data-scale").unwrap().parse().unwrap();
    assert!(after > before);
    deliver_tag(
        &roster,
        "update",
        serde_json::json!({"retracted":[invitations[1]], "asserted":[]}),
        "ui-member-invitations",
    );
    assert_eq!(panel.query_selector_all("line").unwrap().length(), 2);
    assert!(
        panel
            .query_selector("[data-member='did:key:lin'][data-unlinked]")
            .unwrap()
            .is_some()
    );
    deliver(
        &roster,
        "update",
        serde_json::json!({"retracted":[ada], "asserted":[]}),
    );
    assert_eq!(panel.query_selector_all("line").unwrap().length(), 1);
    roster.set_attribute("space", "did:key:other").unwrap();
    assert_eq!(
        panel.query_selector_all(".member-node").unwrap().length(),
        0
    );
    bar.remove();
}

#[dialog_common::test]
async fn panning_shrinks_peripheral_nodes_and_keeps_edges_attached() {
    let (bar, roster) = mount();
    let mut rows: Vec<_> = (0..12)
        .map(|i| member(&format!("m{i}"), &format!("Member {i}")))
        .collect();
    rows[0]["fields"]["role"] = "tonk:founder".into();
    deliver(&roster, "reset", serde_json::json!(rows));
    shadow(&bar, ".space")
        .unchecked_into::<HtmlElement>()
        .click();
    shadow(&bar, ".members")
        .unchecked_into::<HtmlElement>()
        .click();
    let settled = Promise::new(&mut |resolve, _| {
        window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 500)
            .unwrap();
    });
    JsFuture::from(settled).await.unwrap();
    let panel: HtmlElement = shadow(&bar, ".members-list").unchecked_into();
    let root: HtmlElement = shadow(&bar, ".member-space").unchecked_into();
    let dot = shadow(&bar, ".member-space .member-dot");
    let scale = || {
        root.style()
            .get_property_value("--member-scale")
            .unwrap()
            .parse::<f64>()
            .unwrap()
    };
    let centred = scale();
    let full_width = dot.get_bounding_client_rect().width();
    assert!((centred - 1.0).abs() < 0.001);
    let home = panel.scroll_top();
    panel.set_scroll_top(0.0);
    settle().await;
    let peripheral = scale();
    assert!(peripheral < 0.6, "the edge shrinks the root: {peripheral}");
    assert!((dot.get_bounding_client_rect().width() / full_width - peripheral).abs() < 0.01);
    let edge = shadow(&bar, "line[data-from='space']");
    let number =
        |node: &Element, name: &str| node.get_attribute(name).unwrap().parse::<f64>().unwrap();
    let gap = (number(&edge, "x1") - number(&root, "data-graph-x"))
        .hypot(number(&edge, "y1") - number(&root, "data-graph-y"));
    assert!(
        (gap - (15.0 * peripheral + 4.0)).abs() < 0.01,
        "edge follows the smaller disc"
    );
    panel.set_scroll_top(home);
    settle().await;
    assert!(
        (scale() - centred).abs() < 0.001,
        "panning back restores full size"
    );
    assert!((dot.get_bounding_client_rect().width() - full_width).abs() < 0.01);
    // Dragging updates the lens immediately, without waiting for a scroll event.
    let pan = Function::new_with_args(
        "panel",
        "for (const [type,y] of [['pointerdown',90],['pointermove',250],['pointerup',250]]) panel.dispatchEvent(new PointerEvent(type,{bubbles:true,composed:true,pointerId:9,button:0,clientX:100,clientY:y}));",
    );
    pan.call1(&wasm_bindgen::JsValue::NULL, &panel).unwrap();
    assert!(scale() < centred);
    // A live redraw at this pan position must retain the lens rather than flash full-size nodes.
    deliver(
        &roster,
        "update",
        serde_json::json!({"asserted":[member("m1", "Renamed")], "retracted":[]}),
    );
    let updated: HtmlElement = shadow(&bar, ".member-space").unchecked_into();
    assert!(
        updated
            .style()
            .get_property_value("--member-scale")
            .unwrap()
            .parse::<f64>()
            .unwrap()
            < centred
    );
    bar.remove();
}
