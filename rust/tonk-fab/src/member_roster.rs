//! `<ui-member-roster>` — a space's member roster, read live from its own
//! branch and rendered as the FAB's invitation graph.
//!
//! Built on the shared subscribing scaffolding in [`crate::subscribing`]:
//! `shadow() -> false`, an observed `space` attribute, its own stamped
//! `with="main@{did}"`, plain `consumer::subscribe`, bounded retry, and
//! structural frame consumption via `reset`/`update` delegates. See that
//! module's doc for why frame consumption is structural rather than
//! optional — an element that subscribes and never renders is the exact bug
//! this whole scaffolding exists to catch.
//!
//! Reads membership fields through one inline
//! directory-mode predicate (`this` unbound, so every member returns as a
//! row) — see [`crate::logic::member_roster_query_body`]. No concept is
//! named, so nothing seeded on the space's branch is consulted.
//!
//! A second subscription resolves invitation references to their recorded inviter.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::Reflect;
use tonk_common::log;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{CustomEvent, HtmlElement, Response, window};

use crate::logic::{
    member_invitations_query_body, member_roster_query_body, observes_from_repository,
    repository_endpoint, role_manages_members, self_member_did_from_repository,
};
use crate::member_graph::{self, Member};
use crate::shadow::{self, Bound};
use crate::subscribing;

const SUB_TAG: &str = "ui-member-roster";
const INVITATIONS_TAG: &str = "ui-member-invitations";

#[derive(Default)]
pub struct UiMemberRosterElement {
    scaffold: subscribing::Scaffold,
    /// The live member set, keyed by each row's entity `this` so an `update`
    /// delta can upsert/retract individual rows rather than needing a full
    /// snapshot every time. Order is insertion order.
    members: Rc<RefCell<Vec<Member>>>,
    /// The current membership DID, used to mark its row as "you".
    viewer: Rc<RefCell<Option<String>>>,
    invitations: Rc<RefCell<BTreeMap<String, String>>>,
    viewer_request: Rc<Cell<u64>>,
    listeners: Vec<Bound>,
    resize_observer: Option<web_sys::ResizeObserver>,
    scroll_listener: Rc<RefCell<Option<Bound>>>,
    resize_callback: Option<Closure<dyn FnMut()>>,
}

impl CustomElement for UiMemberRosterElement {
    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["space"]
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        let behaviour: Rc<dyn subscribing::Subscribing> = Rc::new(MemberRosterBehaviour {
            members: self.members.clone(),
            viewer: self.viewer.clone(),
            invitations: self.invitations.clone(),
            viewer_request: self.viewer_request.clone(),
        });
        render_rows(
            this,
            &self.members.borrow(),
            self.viewer.borrow().as_deref(),
            &self.invitations.borrow(),
        );
        self.scaffold.connect_all(
            this,
            vec![
                behaviour,
                Rc::new(InvitationBehaviour {
                    members: self.members.clone(),
                    viewer: self.viewer.clone(),
                    invitations: self.invitations.clone(),
                }),
            ],
        );
        resolve_viewer(
            this,
            self.members.clone(),
            self.viewer.clone(),
            self.viewer_request.clone(),
            self.invitations.clone(),
        );
        // Light children connect before the bar builds its shadow buttons.
        // Delegate through the stable host so reconnects cannot leave dead controls.
        if let Some(bar) = this.closest("tonk-fab").ok().flatten() {
            self.listeners.extend(install_map_interactions(&bar, this));
            let resize_host = this.clone();
            let scroll_listener = self.scroll_listener.clone();
            let callback = Closure::wrap(Box::new(move || {
                if let Some(panel) = member_panel(&resize_host) {
                    if scroll_listener.borrow().is_none() {
                        let viewport = panel.clone();
                        *scroll_listener.borrow_mut() =
                            Some(shadow::bind(&panel, "scroll", move |_| {
                                apply_fisheye(&viewport)
                            }));
                    }
                    center_graph(&panel);
                    apply_fisheye(&panel);
                }
            }) as Box<dyn FnMut()>);
            if let Ok(observer) = web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref()) {
                observer.observe(&bar);
                self.resize_observer = Some(observer);
                self.resize_callback = Some(callback);
            }

            let host = this.clone();
            let members = self.members.clone();
            let viewer = self.viewer.clone();
            let invitations = self.invitations.clone();
            self.listeners
                .push(shadow::bind(&bar, "click", move |event| {
                    let factor = event
                        .composed_path()
                        .iter()
                        .filter_map(|v| v.dyn_into::<web_sys::Element>().ok())
                        .find_map(|element| {
                            [
                                ("members-zoom-in", 1.4),
                                ("members-zoom-out", 1.0 / 1.4),
                                ("members-fit", 0.0),
                                ("members", -1.0),
                            ]
                            .into_iter()
                            .find(|(class, _)| element.class_list().contains(class))
                            .map(|(_, factor)| factor)
                        });
                    let Some(factor) = factor else { return };
                    if let Some(panel) = member_panel(&host) {
                        if factor <= 0.0 {
                            let _ = panel.remove_attribute("data-zoom");
                            let _ = panel.toggle_attribute_with_force("data-fit", factor == 0.0);
                        } else {
                            let zoom = panel
                                .get_attribute("data-scale")
                                .and_then(|v| v.parse::<f64>().ok())
                                .unwrap_or(1.0);
                            let _ = panel.set_attribute(
                                "data-zoom",
                                &(zoom * factor).clamp(0.1, 2.0).to_string(),
                            );
                        }
                        render_rows(
                            &host,
                            &members.borrow(),
                            viewer.borrow().as_deref(),
                            &invitations.borrow(),
                        );
                        center_graph(&panel);
                        apply_fisheye(&panel);
                    }
                }));
        }
        if let Some(win) = window() {
            let host = this.clone();
            let members = self.members.clone();
            let viewer = self.viewer.clone();
            let request = self.viewer_request.clone();
            let invitations = self.invitations.clone();
            self.listeners
                .push(shadow::bind(&win, "tonk:task-closed", move |event| {
                    if event
                        .dyn_ref::<CustomEvent>()
                        .and_then(|event| Reflect::get(&event.detail(), &"result".into()).ok())
                        .and_then(|value| value.as_string())
                        .as_deref()
                        == Some("completed")
                    {
                        resolve_viewer(
                            &host,
                            members.clone(),
                            viewer.clone(),
                            request.clone(),
                            invitations.clone(),
                        );
                    }
                }));
        }
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        name: String,
        old: Option<String>,
        new: Option<String>,
    ) {
        if name != "space" || old == new {
            return;
        }
        // The space landed (or moved): the roster subscription was opened
        // against the old value — or skipped entirely while it was blank.
        // Drop it and subscribe against the space that is actually here.
        self.scaffold.disconnect();
        self.members.borrow_mut().clear();
        self.viewer.borrow_mut().take();
        self.invitations.borrow_mut().clear();
        if let Some(panel) = member_panel(this) {
            let _ = panel.remove_attribute("data-zoom");
            let _ = panel.remove_attribute("data-fit");
        }
        render_rows(this, &[], None, &self.invitations.borrow());
        let behaviour: Rc<dyn subscribing::Subscribing> = Rc::new(MemberRosterBehaviour {
            members: self.members.clone(),
            viewer: self.viewer.clone(),
            invitations: self.invitations.clone(),
            viewer_request: self.viewer_request.clone(),
        });
        self.scaffold.connect_all(
            this,
            vec![
                behaviour,
                Rc::new(InvitationBehaviour {
                    members: self.members.clone(),
                    viewer: self.viewer.clone(),
                    invitations: self.invitations.clone(),
                }),
            ],
        );
        resolve_viewer(
            this,
            self.members.clone(),
            self.viewer.clone(),
            self.viewer_request.clone(),
            self.invitations.clone(),
        );
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.viewer_request
            .set(self.viewer_request.get().wrapping_add(1));
        self.listeners.clear();
        self.scroll_listener.borrow_mut().take();
        if let Some(observer) = self.resize_observer.take() {
            observer.disconnect();
        }
        self.resize_callback.take();
        self.scaffold.disconnect();
        if let Some(panel) = member_panel(_this) {
            panel.set_text_content(None);
        }
    }
}

/// This element's [`subscribing::Subscribing`] behaviour: the directory-mode
/// roster query, and rendering delivered frames as member rows.
struct MemberRosterBehaviour {
    invitations: Rc<RefCell<BTreeMap<String, String>>>,
    members: Rc<RefCell<Vec<Member>>>,
    viewer: Rc<RefCell<Option<String>>>,
    viewer_request: Rc<Cell<u64>>,
}

impl MemberRosterBehaviour {
    /// Look the viewer up again when members arrive and none of them is
    /// known to be the viewer yet. The lookup at connect can run before
    /// the space has replicated (a visitor's bar mounts while the space is
    /// still opening), when it finds no members and so no one to be; the
    /// roster arriving is the moment the answer changes.
    fn resolve_viewer_if_unknown(&self, host: &HtmlElement) {
        if self.viewer.borrow().is_none() && !self.members.borrow().is_empty() {
            resolve_viewer(
                host,
                self.members.clone(),
                self.viewer.clone(),
                self.viewer_request.clone(),
                self.invitations.clone(),
            );
        }
    }
}

impl subscribing::Subscribing for MemberRosterBehaviour {
    fn query_body(&self, _this: &HtmlElement) -> Result<String, String> {
        // Directory mode binds no subject — the query reads every member row
        // on whichever branch `with` (stamped from `space` by the
        // scaffolding's default `resolve_with`) points at.
        Ok(member_roster_query_body())
    }

    fn render_reset(&self, host: &HtmlElement, payload: &JsValue) {
        let conclusions = js_sys::Array::from(payload);
        let mut members = self.members.borrow_mut();
        members.clear();
        for i in 0..conclusions.length() {
            if let Some(row) = read_row(&conclusions.get(i)) {
                members.push(row);
            }
        }
        render_rows(
            host,
            &members,
            self.viewer.borrow().as_deref(),
            &self.invitations.borrow(),
        );
        drop(members);
        self.resolve_viewer_if_unknown(host);
    }

    fn render_update(&self, host: &HtmlElement, payload: &JsValue) {
        let retracted = Reflect::get(payload, &"retracted".into()).unwrap_or(JsValue::UNDEFINED);
        let asserted = Reflect::get(payload, &"asserted".into()).unwrap_or(JsValue::UNDEFINED);
        let mut members = self.members.borrow_mut();

        let retracted_rows = js_sys::Array::from(&retracted);
        for i in 0..retracted_rows.length() {
            if let Some(row) = read_row(&retracted_rows.get(i)) {
                members.retain(|existing| existing.this != row.this);
            }
        }

        let asserted_rows = js_sys::Array::from(&asserted);
        for i in 0..asserted_rows.length() {
            if let Some(row) = read_row(&asserted_rows.get(i)) {
                match members
                    .iter_mut()
                    .find(|existing| existing.this == row.this)
                {
                    Some(existing) => *existing = row,
                    None => members.push(row),
                }
            }
        }

        render_rows(
            host,
            &members,
            self.viewer.borrow().as_deref(),
            &self.invitations.borrow(),
        );
        drop(members);
        self.resolve_viewer_if_unknown(host);
    }

    fn tag(&self) -> &'static str {
        SUB_TAG
    }
}

/// Read a member off a raw subscription row. `None` for a missing/empty row,
/// a missing entity id, or any missing required string field.
fn read_row(row: &JsValue) -> Option<Member> {
    if row.is_undefined() || row.is_null() {
        return None;
    }
    let this_id = Reflect::get(row, &"this".into()).ok()?.as_string()?;
    let fields = Reflect::get(row, &"fields".into()).ok()?;
    let field = |name: &str| {
        Reflect::get(&fields, &JsValue::from_str(name))
            .ok()
            .and_then(|value| value.as_string())
    };
    Some(Member {
        this: this_id,
        name: field("name")?,
        did: field("member")?,
        role: field("role")?,
        invitation: field("invitation"),
    })
}

/// Keep the stack compact, with the full roster in the modal body.
fn member_panel(host: &HtmlElement) -> Option<HtmlElement> {
    host.closest("tonk-fab")
        .ok()
        .flatten()
        .and_then(|bar| bar.shadow_root())
        .and_then(|root| root.query_selector(".members-list").ok().flatten())
        .and_then(|panel| panel.dyn_into::<HtmlElement>().ok())
}

/// Separate provenance frames can arrive before or after membership frames.
struct InvitationBehaviour {
    invitations: Rc<RefCell<BTreeMap<String, String>>>,
    members: Rc<RefCell<Vec<Member>>>,
    viewer: Rc<RefCell<Option<String>>>,
}

impl InvitationBehaviour {
    fn apply(&self, payload: &JsValue, retract: bool) {
        for row in js_sys::Array::from(payload).iter() {
            let Some(id) = Reflect::get(&row, &"this".into())
                .ok()
                .and_then(|v| v.as_string())
            else {
                continue;
            };
            let mut invitations = self.invitations.borrow_mut();
            if retract {
                invitations.remove(&id);
            } else if let Some(inviter) = Reflect::get(&row, &"fields".into())
                .ok()
                .and_then(|fields| Reflect::get(&fields, &"inviter".into()).ok())
                .and_then(|v| v.as_string())
            {
                invitations.insert(id, inviter);
            }
        }
    }
    fn render(&self, host: &HtmlElement) {
        render_rows(
            host,
            &self.members.borrow(),
            self.viewer.borrow().as_deref(),
            &self.invitations.borrow(),
        );
    }
}

impl subscribing::Subscribing for InvitationBehaviour {
    fn query_body(&self, _: &HtmlElement) -> Result<String, String> {
        Ok(member_invitations_query_body())
    }
    fn tag(&self) -> &'static str {
        INVITATIONS_TAG
    }
    fn render_reset(&self, host: &HtmlElement, payload: &JsValue) {
        self.invitations.borrow_mut().clear();
        self.apply(payload, false);
        self.render(host);
    }
    fn render_update(&self, host: &HtmlElement, payload: &JsValue) {
        for (key, retract) in [("retracted", true), ("asserted", false)] {
            if let Ok(rows) = Reflect::get(payload, &key.into()) {
                self.apply(&rows, retract);
            }
        }
        self.render(host);
    }
}

fn render_rows(
    host: &HtmlElement,
    members: &[Member],
    viewer: Option<&str>,
    invitations: &BTreeMap<String, String>,
) {
    stamp_manages(host, members, viewer);
    let Some(panel) = member_panel(host) else {
        return;
    };
    let scroll = (panel.scroll_left(), panel.scroll_top());
    let selected = panel
        .query_selector("[aria-pressed=true]")
        .ok()
        .flatten()
        .and_then(|n| n.get_attribute("data-member"));
    let focused = panel
        .owner_document()
        .and_then(|_| host.closest("tonk-fab").ok().flatten())
        .and_then(|bar| bar.shadow_root())
        .and_then(|root| root.active_element())
        .and_then(|node| node.get_attribute("data-member"));
    panel.set_text_content(None);
    let Some(document) = window().and_then(|window| window.document()) else {
        return;
    };
    let element = |tag: &str, class: &str, text: &str| {
        let node = document.create_element(tag).expect("valid HTML tag");
        node.set_class_name(class);
        node.set_text_content(Some(text));
        node
    };
    if members.is_empty() {
        show_member_detail(&panel, None);
        if let Some(count) = panel
            .parent_element()
            .and_then(|section| section.query_selector(".member-count").ok().flatten())
        {
            count.set_text_content(Some("0"));
        }
        let _ = panel.append_child(&element("p", "members-empty", "no members are available"));
        return;
    }
    let graph = member_graph::layout(members, invitations);
    if let Some(section) = panel.parent_element() {
        if let Some(count) = section.query_selector(".member-count").ok().flatten() {
            count.set_text_content(Some(&graph.nodes.len().to_string()));
        }
        if let Some(detail) = section.query_selector(".member-detail").ok().flatten() {
            let _ = detail.set_attribute("hidden", "");
        }
    }
    let fit_width = if panel.client_width() > 0 {
        panel.client_width() as f64 - 16.0
    } else {
        320.0
    };
    let fit_height = if panel.client_height() > 0 {
        panel.client_height() as f64 - 16.0
    } else {
        260.0
    };
    let scale = panel
        .get_attribute("data-zoom")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or_else(|| {
            let fit = (fit_width.min(fit_height) / graph.size).min(1.0);
            if panel.has_attribute("data-fit") {
                fit
            } else {
                fit.max(0.75)
            }
        });
    let _ = panel.set_attribute("data-scale", &scale.to_string());
    let viewport = element("div", "member-graph-viewport", "");
    let _ = viewport.set_attribute(
        "style",
        &format!(
            "width:{}px;height:{}px",
            graph.size * scale,
            graph.size * scale
        ),
    );

    let canvas = element("div", "member-graph", "");
    let _ = canvas.set_attribute(
        "style",
        &format!(
            "width:{}px;height:{}px;transform:scale({scale});transform-origin:top left",
            graph.size, graph.size
        ),
    );
    let svg = document
        .create_element_ns(Some("http://www.w3.org/2000/svg"), "svg")
        .unwrap();
    let _ = svg.set_attribute("viewBox", &format!("0 0 {0} {0}", graph.size));
    let _ = svg.set_attribute("aria-hidden", "true");
    let _ = svg.set_attribute("class", "member-edges");
    // The static marker contains no user content.
    svg.set_inner_html(r#"<defs><marker id="member-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse"><path d="M 0 0 L 10 5 L 0 10 z" fill="currentColor"/></marker></defs>"#);
    for node in &graph.nodes {
        let Some(parent) = node.parent else { continue };
        let (x, y) = if parent == 0 {
            (graph.size / 2.0, graph.size / 2.0)
        } else {
            (graph.nodes[parent - 1].x, graph.nodes[parent - 1].y)
        };
        let (dx, dy) = (node.x - x, node.y - y);
        // Connections meet the discs, rather than their floating labels.
        let radius = if viewer == Some(node.member.did.as_str()) {
            16.0
        } else {
            13.0
        };
        let clip = radius / dx.hypot(dy);
        let start_radius = if parent == 0 {
            19.0
        } else if viewer == Some(graph.nodes[parent - 1].member.did.as_str()) {
            16.0
        } else {
            13.0
        };
        let start_clip = start_radius / dx.hypot(dy);
        let line = document
            .create_element_ns(Some("http://www.w3.org/2000/svg"), "line")
            .unwrap();
        for (key, value) in [
            ("x1", x + dx * start_clip),
            ("y1", y + dy * start_clip),
            ("x2", node.x - dx * clip),
            ("y2", node.y - dy * clip),
        ] {
            let _ = line.set_attribute(key, &value.to_string());
        }
        let _ = line.set_attribute(
            "data-from",
            if parent == 0 {
                "space"
            } else {
                &graph.nodes[parent - 1].member.did
            },
        );
        let _ = line.set_attribute("data-to", &node.member.did);
        let _ = line.set_attribute("marker-end", "url(#member-arrow)");
        let _ = svg.append_child(&line);
    }
    let _ = canvas.append_child(&svg);
    let space = element("div", "member-node member-space", "");
    stamp_geometry(&space, "space", graph.size / 2.0, graph.size / 2.0, 15.0);
    let dot = element("span", "member-dot", "");
    let _ = dot.set_attribute("aria-hidden", "true");
    let _ = space.append_child(&dot);
    let _ = space.append_child(&element("span", "member-caption", "space"));
    let _ = space.set_attribute("style", &format!("left:{0}px;top:{0}px", graph.size / 2.0));
    let _ = canvas.append_child(&space);
    for node in &graph.nodes {
        let member = node.member;
        let row = element("button", "member-node mem-row", "");
        let _ = row.set_attribute("type", "button");
        let _ = row.set_attribute(
            "aria-pressed",
            if selected.as_deref() == Some(member.did.as_str()) {
                "true"
            } else {
                "false"
            },
        );
        if node.x < graph.size / 2.0 {
            let _ = row.set_attribute("data-label-left", "");
        }
        if viewer == Some(&member.did) {
            let _ = row.set_attribute("data-self", "");
        }
        let dot = element("span", "member-dot", "");
        let _ = dot.set_attribute("aria-hidden", "true");
        let _ = row.append_child(&dot);
        let _ = row.set_attribute("data-member", &member.did);
        stamp_geometry(
            &row,
            &member.did,
            node.x,
            node.y,
            if viewer == Some(&member.did) {
                12.0
            } else {
                9.0
            },
        );
        let _ = row.set_attribute("style", &format!("left:{}px;top:{}px", node.x, node.y));
        let _ = row.set_attribute("tabindex", "0");
        let role = match member.role.as_str() {
            "tonk:founder" => "owner",
            "tonk:admin" => "admin",
            _ => "member",
        };
        let relation = match node.parent {
            Some(0) => "owner of this space".to_string(),
            Some(parent) => format!("invited by {}", graph.nodes[parent - 1].member.name),
            None => "invitation history unavailable".to_string(),
        };
        let description = format!(
            "{} · {}{} · {}{}",
            member.name,
            role,
            if viewer == Some(&member.did) {
                " · you"
            } else {
                ""
            },
            relation,
            if !node.rooted && node.parent.is_some() {
                " · path to space unavailable"
            } else {
                ""
            }
        );
        let _ = row.set_attribute("aria-label", &description);
        let _ = row.set_attribute("title", &description);

        if !node.rooted {
            let _ = row.set_attribute("data-unlinked", "");
        }
        let name = element(
            "span",
            if viewer == Some(&member.did) {
                "mem-name mem-self"
            } else {
                "mem-name"
            },
            &member.name,
        );
        let caption = element("span", "member-caption", "");
        let _ = caption.append_child(&name);
        let tags = element("span", "mem-tags", "");
        if role != "member" {
            let _ = tags.append_child(&element("span", "mem-tag mem-role", role));
        }
        if viewer == Some(&member.did) {
            let _ = tags.append_child(&element("span", "mem-tag mem-you", "you"));
        }
        let _ = caption.append_child(&tags);
        let _ = row.append_child(&caption);
        let _ = canvas.append_child(&row);
        if focused.as_deref() == Some(member.did.as_str()) {
            let _ = row.set_attribute("data-restore-focus", "");
        }
    }
    let _ = viewport.append_child(&canvas);
    let _ = panel.append_child(&viewport);
    if let Some(focus) = panel
        .query_selector("[data-restore-focus]")
        .ok()
        .flatten()
        .and_then(|e| e.dyn_into::<HtmlElement>().ok())
    {
        let _ = focus.focus();
    }
    if let Some(selected) = panel.query_selector("[aria-pressed=true]").ok().flatten() {
        show_member_detail(&panel, selected.get_attribute("aria-label").as_deref());
    }
    panel.set_scroll_left(scroll.0);
    panel.set_scroll_top(scroll.1);
    apply_fisheye(&panel);
}

fn stamp_geometry(node: &web_sys::Element, id: &str, x: f64, y: f64, radius: f64) {
    let _ = node.set_attribute("data-graph-id", id);
    for (name, value) in [
        ("data-graph-x", x),
        ("data-graph-y", y),
        ("data-graph-radius", radius),
    ] {
        let _ = node.set_attribute(name, &value.to_string());
    }
}

fn mobile_roster() -> bool {
    window()
        .and_then(|win| win.inner_width().ok())
        .and_then(|width| width.as_f64())
        .is_some_and(|width| width <= 640.0)
}

fn center_graph(panel: &HtmlElement) {
    if mobile_roster() {
        panel.set_scroll_left(0.0);
        // Enter the list at its start, but preserve native scrolling when details resize it.
        if !panel.has_attribute("data-mobile-roster") {
            panel.set_scroll_top(0.0);
        }
        let _ = panel.set_attribute("data-mobile-roster", "");
    } else {
        let _ = panel.remove_attribute("data-mobile-roster");
        panel.set_scroll_left(f64::from(panel.scroll_width() - panel.client_width()) / 2.0);
        panel.set_scroll_top(f64::from(panel.scroll_height() - panel.client_height()) / 2.0);
    }
}

/// Scale about each fixed graph position, so panning never changes layout or
/// scroll extents. Read viewport geometry once before writing node styles.
fn apply_fisheye(panel: &HtmlElement) {
    if mobile_roster() {
        return;
    }
    let width = f64::from(panel.client_width()) / 2.0;
    let height = f64::from(panel.client_height()) / 2.0;
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let Some(canvas) = panel.query_selector(".member-graph").ok().flatten() else {
        return;
    };
    let panel_rect = panel.get_bounding_client_rect();
    let canvas_rect = canvas.get_bounding_client_rect();
    let zoom = panel
        .get_attribute("data-scale")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(1.0);
    let number = |node: &web_sys::Element, attribute: &str| {
        node.get_attribute(attribute)
            .and_then(|v| v.parse::<f64>().ok())
    };
    let mut geometry = BTreeMap::new();
    if let Ok(nodes) = canvas.query_selector_all("[data-graph-id]") {
        for index in 0..nodes.length() {
            let Some(node) = nodes
                .item(index)
                .and_then(|n| n.dyn_into::<HtmlElement>().ok())
            else {
                continue;
            };
            let (Some(id), Some(x), Some(y), Some(radius)) = (
                node.get_attribute("data-graph-id"),
                number(&node, "data-graph-x"),
                number(&node, "data-graph-y"),
                number(&node, "data-graph-radius"),
            ) else {
                continue;
            };
            let scale = member_graph::peripheral_scale(
                (canvas_rect.x() + x * zoom - panel_rect.x() - width) / width,
                (canvas_rect.y() + y * zoom - panel_rect.y() - height) / height,
            );
            let _ = node
                .style()
                .set_property("--member-scale", &scale.to_string());
            geometry.insert(id, (x, y, radius * scale + 4.0));
        }
    }
    if let Ok(edges) = canvas.query_selector_all("line[data-from][data-to]") {
        for index in 0..edges.length() {
            let Some(edge) = edges
                .item(index)
                .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
            else {
                continue;
            };
            let Some(from) = edge
                .get_attribute("data-from")
                .and_then(|id| geometry.get(&id))
            else {
                continue;
            };
            let Some(to) = edge
                .get_attribute("data-to")
                .and_then(|id| geometry.get(&id))
            else {
                continue;
            };
            let (dx, dy) = (to.0 - from.0, to.1 - from.1);
            let length = dx.hypot(dy).max(1.0);
            for (name, value) in [
                ("x1", from.0 + dx * from.2 / length),
                ("y1", from.1 + dy * from.2 / length),
                ("x2", to.0 - dx * to.2 / length),
                ("y2", to.1 - dy * to.2 / length),
            ] {
                let _ = edge.set_attribute(name, &value.to_string());
            }
        }
    }
    // Scroll events arrive during a rendering update, after the scroll that
    // caused them; say when the lens has followed.
    shadow::emit(panel, "fabb-lens", &JsValue::NULL);
}

/// Delegate from the stable host: the shadow buttons are built after this
/// subscribing child connects, and graph nodes are replaced by live frames.
fn install_map_interactions(bar: &web_sys::Element, host: &HtmlElement) -> Vec<Bound> {
    type Pan = (i32, f64, f64, f64, f64);
    let pan: Rc<RefCell<Option<Pan>>> = Rc::default();
    let dragged = Rc::new(Cell::new(false));
    let mut listeners = Vec::new();
    let active_pan = pan.clone();
    let did_drag = dragged.clone();
    let source = host.clone();
    listeners.push(shadow::bind(bar, "pointerdown", move |event| {
        let Some(pointer) = event.dyn_ref::<web_sys::PointerEvent>() else {
            return;
        };
        if mobile_roster()
            || pointer.button() != 0
            || !event.composed_path().iter().any(|item| {
                item.dyn_ref::<web_sys::Element>()
                    .is_some_and(|e| e.class_list().contains("members-list"))
            })
        {
            return;
        }
        if let Some(panel) = member_panel(&source) {
            did_drag.set(false);
            *active_pan.borrow_mut() = Some((
                pointer.pointer_id(),
                pointer.client_x(),
                pointer.client_y(),
                panel.scroll_left(),
                panel.scroll_top(),
            ));
        }
    }));
    let active_pan = pan.clone();
    let did_drag = dragged.clone();
    let source = host.clone();
    let capture = bar.clone();
    listeners.push(shadow::bind(bar, "pointermove", move |event| {
        let Some(pointer) = event.dyn_ref::<web_sys::PointerEvent>() else {
            return;
        };
        let Some((id, x, y, left, top)) = *active_pan.borrow() else {
            return;
        };
        if id != pointer.pointer_id() {
            return;
        }
        let (dx, dy) = (pointer.client_x() - x, pointer.client_y() - y);
        if !did_drag.get() && dx.hypot(dy) < 4.0 {
            return;
        }
        did_drag.set(true);
        let _ = capture.set_pointer_capture(id);
        if let Some(panel) = member_panel(&source) {
            let _ = panel.set_attribute("data-panning", "");
            panel.set_scroll_left(left - dx);
            panel.set_scroll_top(top - dy);
            apply_fisheye(&panel);
        }
        event.prevent_default();
    }));
    for name in ["pointerup", "pointercancel", "lostpointercapture"] {
        let active_pan = pan.clone();
        let source = host.clone();
        let capture = bar.clone();
        listeners.push(shadow::bind(bar, name, move |_| {
            let previous = active_pan.borrow_mut().take();
            if let Some((id, ..)) = previous {
                let _ = capture.release_pointer_capture(id);
            }
            if let Some(panel) = member_panel(&source) {
                let _ = panel.remove_attribute("data-panning");
            }
        }));
    }
    let source = host.clone();
    listeners.push(shadow::bind(bar, "click", move |event| {
        if dragged.replace(false) {
            return;
        }
        let Some(panel) = member_panel(&source) else {
            return;
        };
        let selected = event
            .composed_path()
            .iter()
            .filter_map(|item| item.dyn_into::<web_sys::Element>().ok())
            .find(|element| element.has_attribute("data-member"));
        let Some(selected) = selected else { return };
        let was_selected = selected.get_attribute("aria-pressed").as_deref() == Some("true");
        if let Ok(nodes) = panel.query_selector_all("[aria-pressed]") {
            for index in 0..nodes.length() {
                if let Some(node) = nodes
                    .item(index)
                    .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
                {
                    let _ = node.set_attribute("aria-pressed", "false");
                }
            }
        }
        let _ = selected.set_attribute("aria-pressed", if was_selected { "false" } else { "true" });
        show_member_detail(
            &panel,
            if was_selected {
                None
            } else {
                selected.get_attribute("aria-label")
            }
            .as_deref(),
        );
    }));
    listeners
}

fn show_member_detail(panel: &HtmlElement, description: Option<&str>) {
    if let Some(detail) = panel
        .parent_element()
        .and_then(|section| section.query_selector(".member-detail").ok().flatten())
    {
        detail.set_text_content(description);
        let _ = detail.toggle_attribute_with_force("hidden", description.is_none());
        apply_fisheye(panel);
    }
}

/// Resolve the current member from the repository's `is_self` projection.
/// Its DID is the account principal that owns the membership, which need not
/// be this device's profile DID. Repaint rows delivered during the request.
/// Stamp the bar with whether the viewer runs the space (`data-manages`),
/// which is what offers it publishing: the worker refuses anyone else, and a
/// reader of a published space must not be offered to make it private.
fn stamp_manages(host: &HtmlElement, members: &[Member], viewer: Option<&str>) {
    let Some(bar) = host.closest("tonk-fab").ok().flatten() else {
        return;
    };
    let manages = viewer.is_some_and(|viewer| {
        members
            .iter()
            .any(|member| member.did == viewer && role_manages_members(&member.role))
    });
    if manages {
        let _ = bar.set_attribute("data-manages", "");
    } else {
        let _ = bar.remove_attribute("data-manages");
    }
}

/// Stamp the bar with whether the viewer only observes the space
/// (`data-observer`): the bar says so beside the space's name, and leaves
/// out what a reader cannot do.
fn stamp_observer(host: &HtmlElement, observes: bool) {
    if let Some(bar) = host.closest("tonk-fab").ok().flatten() {
        let _ = bar.toggle_attribute_with_force("data-observer", observes);
    }
}

fn resolve_viewer(
    host: &HtmlElement,
    members: Rc<RefCell<Vec<Member>>>,
    viewer: Rc<RefCell<Option<String>>>,
    request: Rc<Cell<u64>>,
    invitations: Rc<RefCell<BTreeMap<String, String>>>,
) {
    let Some(win) = window() else { return };
    let Some(space) = host.get_attribute("space") else {
        return;
    };
    let Ok(endpoint) = repository_endpoint(&space) else {
        return;
    };
    let current = request.get().wrapping_add(1);
    request.set(current);
    viewer.borrow_mut().take();
    stamp_observer(host, false);
    render_rows(host, &members.borrow(), None, &invitations.borrow());

    let host = host.clone();
    spawn_local(async move {
        let response = match JsFuture::from(win.fetch_with_str(&endpoint)).await {
            Ok(response) => response.dyn_into::<Response>().ok(),
            Err(error) => {
                log!("ui-member-roster repository lookup failed: {error:?}");
                return;
            }
        };
        let Some(response) = response.filter(Response::ok) else {
            return;
        };
        let Ok(promise) = response.json() else {
            return;
        };
        let Ok(info) = JsFuture::from(promise).await else {
            return;
        };
        let Some(json) = js_sys::JSON::stringify(&info)
            .ok()
            .and_then(|json| json.as_string())
        else {
            return;
        };
        let Ok(info) = serde_json::from_str::<serde_json::Value>(&json) else {
            return;
        };
        if request.get() != current
            || !host.is_connected()
            || host.get_attribute("space").as_deref() != Some(space.as_str())
        {
            return;
        }
        *viewer.borrow_mut() = self_member_did_from_repository(&info);
        stamp_observer(&host, observes_from_repository(&info));
        render_rows(
            &host,
            &members.borrow(),
            viewer.borrow().as_deref(),
            &invitations.borrow(),
        );
    });
}

/// Register `<ui-member-roster>`. Idempotent.
pub fn register() {
    if subscribing::already_registered(SUB_TAG) {
        return;
    }
    UiMemberRosterElement::define(SUB_TAG);
    subscribing::install_frame_shims(SUB_TAG);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscribing::Subscribing;

    #[wasm_bindgen_test::wasm_bindgen_test]
    fn roster_updates_names_and_viewer_labels() {
        crate::register();
        let document = window().unwrap().document().unwrap();
        let bar: HtmlElement = document
            .create_element("tonk-fab")
            .unwrap()
            .unchecked_into();
        bar.set_attribute("space", "did:key:members").unwrap();
        document.body().unwrap().append_child(&bar).unwrap();
        let host: HtmlElement = bar
            .query_selector("ui-member-roster")
            .unwrap()
            .unwrap()
            .unchecked_into();
        let panel: HtmlElement = bar
            .shadow_root()
            .unwrap()
            .query_selector(".members-list")
            .unwrap()
            .unwrap()
            .unchecked_into();
        let behaviour = MemberRosterBehaviour {
            invitations: Rc::default(),
            members: Rc::default(),
            viewer: Rc::new(RefCell::new(Some("did:key:owner".into()))),
            viewer_request: Rc::default(),
        };
        let owner = serde_json::json!({ "this": "owner-membership", "fields": {
            "name": "<Owner>", "member": "did:key:owner", "role": "tonk:founder"
        }});
        let member = serde_json::json!({ "this": "member-membership", "fields": {
            "name": "Member", "member": "did:key:member", "role": "tonk:member"
        }});
        let js = |value: serde_json::Value| js_sys::JSON::parse(&value.to_string()).unwrap();
        behaviour.render_reset(&host, &js(serde_json::json!([owner])));
        assert_eq!(
            bar.shadow_root()
                .unwrap()
                .query_selector(".members span")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("view members")
        );
        assert_eq!(
            panel
                .query_selector(".mem-self")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("<Owner>")
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
                .query_selector(".mem-role")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("owner")
        );
        assert!(
            panel.query_selector("owner").unwrap().is_none(),
            "names remain plain text"
        );
        behaviour.render_update(
            &host,
            &js(serde_json::json!({ "asserted": [member], "retracted": [] })),
        );
        assert_eq!(
            bar.shadow_root()
                .unwrap()
                .query_selector(".members span")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("view members")
        );
        *behaviour.viewer.borrow_mut() = Some("did:key:member".into());
        render_rows(
            &host,
            &behaviour.members.borrow(),
            behaviour.viewer.borrow().as_deref(),
            &behaviour.invitations.borrow(),
        );
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
        behaviour.render_update(
            &host,
            &js(serde_json::json!({ "asserted": [], "retracted": [owner] })),
        );
        assert_eq!(panel.query_selector_all(".mem-row").unwrap().length(), 1);
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
        bar.remove();
    }
}
