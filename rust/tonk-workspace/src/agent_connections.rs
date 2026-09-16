//! Settings projection of public issued grants and acknowledged revocations.
//! These rows never establish authority or imply live agent presence.

use tonk_worker_api::AgentConnectionSummary;
use wasm_bindgen::JsCast as _;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, HtmlElement};

fn status(host: &HtmlElement, text: &str) {
    if let Ok(Some(node)) = host.query_selector("[data-connections-status]") {
        node.set_text_content(Some(text));
    }
}

fn generation(host: &HtmlElement) -> String {
    let next = host
        .get_attribute("data-connections-generation")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
        .wrapping_add(1)
        .to_string();
    let _ = host.set_attribute("data-connections-generation", &next);
    next
}

fn current(host: &HtmlElement, expected: &str) -> bool {
    host.is_connected()
        && host.get_attribute("data-connections-generation").as_deref() == Some(expected)
}

/// Load current-account records. A feature-disabled worker has no endpoint.
pub(crate) fn refresh(host: &HtmlElement) {
    crate::terminal_connections::refresh(host);
    let expected = generation(host);
    if let Ok(Some(list)) = host.query_selector("[data-connections-list]") {
        list.set_text_content(None);
    }
    status(host, "loading access records…");
    let host = host.clone();
    spawn_local(async move {
        let result = tonk_host::get_json("/api/account/connections")
            .await
            .ok()
            .and_then(|body| serde_json::from_str::<Vec<AgentConnectionSummary>>(&body).ok());
        if !current(&host, &expected) {
            return;
        }
        let Some(groups) = result else {
            status(
                &host,
                "access records could not be loaded. refresh to try again.",
            );
            return;
        };
        let groups: Vec<_> = groups
            .into_iter()
            .filter(|group| group.request_id.is_none())
            .collect();
        if let Ok(Some(section)) = host.query_selector("[data-agent-connections]") {
            let _ = section.remove_attribute("hidden");
        }
        status(
            &host,
            if groups.is_empty() {
                "no agent invites issued from this account"
            } else {
                "saved grant records; remote access is checked when used"
            },
        );
        if let Ok(Some(list)) = host.query_selector("[data-connections-list]") {
            for group in &groups {
                let _ = append_group(&list, group);
            }
        }
    });
}

fn append_text(parent: &Element, tag: &str, class: &str, value: &str) -> Option<Element> {
    let node = parent.owner_document()?.create_element(tag).ok()?;
    node.set_class_name(class);
    node.set_text_content(Some(value));
    parent.append_child(&node).ok()?;
    Some(node)
}

pub(crate) fn append_group(list: &Element, group: &AgentConnectionSummary) -> Option<()> {
    let row = append_text(list, "article", "connection-record", "")?;
    row.set_attribute("data-connection-id", &group.id).ok()?;
    row.set_attribute("aria-label", &group.label).ok()?;
    let heading = append_text(&row, "div", "srowd", "")?;
    append_text(&heading, "b", "lft", &group.label)?;
    let count = group
        .targets
        .iter()
        .filter(|target| target.acknowledged)
        .count();
    let all_acknowledged = !group.targets.is_empty() && count == group.targets.len();
    let button = append_text(
        &heading,
        "button",
        "cta",
        if !all_acknowledged
            && (group.status == "partial"
                || count > 0
                || group.targets.iter().any(|target| target.error.is_some()))
        {
            "retry revocation"
        } else {
            if group.request_id.is_some() {
                "remove space access"
            } else {
                "revoke invite"
            }
        },
    )?;
    button.set_attribute("type", "button").ok()?;
    button
        .set_attribute("data-connection-revoke", &group.id)
        .ok()?;
    button
        .set_attribute(
            "aria-label",
            &format!(
                "{} {}",
                if group.request_id.is_some() {
                    "remove space access for"
                } else {
                    "revoke invite"
                },
                group.label
            ),
        )
        .ok()?;
    if all_acknowledged {
        button.set_attribute("disabled", "").ok()?;
    }
    append_text(&row, "p", "expl", &format!("space: {}", group.subject))?;
    append_text(
        &row,
        "p",
        "expl",
        &format!("recipient: {}", group.recipient),
    )?;
    append_text(&row, "p", "expl", &group.scope)?;
    let date = js_sys::Date::new_0();
    date.set_time(group.expires_at as f64 * 1000.0);
    let expiry = if date.get_time().is_finite() {
        date.to_iso_string().as_string().unwrap_or_default()
    } else {
        group.expires_at.to_string()
    };
    append_text(&row, "p", "expl", &format!("expires: {expiry}"))?;
    if group.status == "expired" {
        append_text(
            &row,
            "p",
            "expl",
            "expired; new authorization is needed for remote work",
        )?;
    }
    append_text(
        &row,
        "p",
        "expl",
        if group.confirmed {
            "setup confirmation received"
        } else {
            "no setup confirmation recorded"
        },
    )?;
    let message = if count == 0 && group.status == "partial" {
        "revocation requested; no acknowledgement yet".to_string()
    } else if count == 0 {
        "no revocation acknowledged".to_string()
    } else {
        format!(
            "revocation acknowledged for {count} of {} grants",
            group.targets.len()
        )
    };
    append_text(&row, "p", "expl", &message)?;
    if group.targets.iter().any(|target| target.error.is_some()) {
        append_text(
            &row,
            "p",
            "expl",
            "some grants could not be revoked. retry to send the remaining revocations.",
        )?;
    }
    Some(())
}

/// Send the complete grant group to the existing standard revocation path.
pub(crate) fn revoke(host: &HtmlElement, button: &Element) {
    if button.has_attribute("disabled") {
        return;
    }
    let Some(id) = button.get_attribute("data-connection-revoke") else {
        return;
    };
    if id.len() != 64 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return;
    }
    let _ = button.set_attribute("disabled", "");
    status(host, "sending revocations…");
    let expected = host
        .get_attribute("data-connections-generation")
        .unwrap_or_default();
    let host = host.clone();
    let button = button.clone();
    spawn_local(async move {
        let result = tonk_host::post_json(&format!("/api/account/connections/{id}/revoke"), "{}")
            .await
            .ok()
            .and_then(|body| serde_json::from_str::<AgentConnectionSummary>(&body).ok());
        if !current(&host, &expected) {
            return;
        }
        let Some(group) = result else {
            let _ = button.remove_attribute("disabled");
            status(
                &host,
                "revocation could not be confirmed. retry or refresh to check saved acknowledgements.",
            );
            return;
        };
        if group.id != id {
            status(
                &host,
                "the access record changed. refresh before trying again.",
            );
            return;
        }
        if let Ok(Some(row)) = button.closest("[data-connection-id]") {
            // Render into a temporary parent, then replace only this record.
            // Refresh and other groups keep their DOM and focus intact.
            if let Some(document) = row.owner_document()
                && let Ok(container) = document.create_element("div")
            {
                let _ = append_group(&container, &group);
                if let Some(replacement) = container.first_element_child()
                    && let Some(parent) = row.parent_node()
                {
                    let _ = parent.replace_child(&replacement, &row);
                }
            }
        }
        let count = group
            .targets
            .iter()
            .filter(|target| target.acknowledged)
            .count();
        status(
            &host,
            &format!(
                "revocation acknowledged for {count} of {} grants. downloaded data remains with its holders.",
                group.targets.len()
            ),
        );
        if let Ok(Some(node)) = host.query_selector("[data-connections-status]")
            && let Ok(node) = node.dyn_into::<HtmlElement>()
        {
            node.set_tab_index(-1);
            let _ = node.focus();
        }
    });
}
