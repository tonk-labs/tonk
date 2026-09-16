//! Public terminal grant management. Delivery receipts never imply live access.
use tonk_worker_api::{
    AgentConnectionSummary, TerminalConnectionAddReceipt, TerminalConnectionAddRequest,
    TerminalConnectionSummary, TerminalLinkSpaces,
};
use wasm_bindgen::JsCast as _;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, HtmlElement, HtmlInputElement};

fn append(parent: &Element, tag: &str, class: &str, value: &str) -> Option<Element> {
    let node = parent.owner_document()?.create_element(tag).ok()?;
    node.set_class_name(class);
    node.set_text_content(Some(value));
    parent.append_child(&node).ok()?;
    Some(node)
}
fn button(parent: &Element, label: &str, attr: &str) -> Option<Element> {
    let node = append(parent, "button", "cta", label)?;
    node.set_attribute("type", "button").ok()?;
    node.set_attribute(attr, "").ok()?;
    Some(node)
}
fn status(parent: &Element, value: &str) {
    if let Ok(Some(node)) = parent.query_selector("[data-terminal-management-status]") {
        node.set_text_content(Some(value));
    }
}
fn announce(host: &HtmlElement, value: &str) {
    if let Ok(Some(node)) = host.query_selector("[data-terminal-management-result]") {
        node.set_text_content(Some(value));
        if let Ok(node) = node.dyn_into::<HtmlElement>() {
            let _ = node.focus();
        }
    }
}
fn current(host: &HtmlElement, token: &str) -> bool {
    host.is_connected()
        && host
            .get_attribute("data-terminal-management-generation")
            .as_deref()
            == Some(token)
}
pub(crate) fn refresh(host: &HtmlElement) {
    let token = host
        .get_attribute("data-terminal-management-generation")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
        .wrapping_add(1)
        .to_string();
    let _ = host.set_attribute("data-terminal-management-generation", &token);
    let Ok(Some(list)) = host.query_selector("[data-terminal-connections-list]") else {
        return;
    };
    list.set_text_content(Some("loading terminal access…"));
    let host = host.clone();
    spawn_local(async move {
        let rows = tonk_host::get_json("/api/account/terminal-links")
            .await
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<TerminalConnectionSummary>>(&s).ok());
        if !current(&host, &token) {
            return;
        }
        let Some(rows) = rows else {
            list.set_text_content(Some(
                "terminal access could not be loaded. refresh to try again.",
            ));
            return;
        };
        if let Ok(Some(section)) = host.query_selector("[data-terminal-connections]") {
            let _ = section.remove_attribute("hidden");
        }
        list.set_text_content(None);
        if rows.is_empty() {
            list.set_text_content(Some("no terminals linked from this account"));
        }
        for row in &rows {
            let _ = render(&list, row);
        }
    });
}
fn render(parent: &Element, terminal: &TerminalConnectionSummary) -> Option<()> {
    let row = append(parent, "article", "connection-record", "")?;
    row.set_attribute("data-terminal-record", &terminal.request_id)
        .ok()?;
    row.set_attribute("data-terminal-account", &terminal.account)
        .ok()?;
    row.set_attribute(
        "data-terminal-groups",
        &serde_json::to_string(&terminal.spaces).ok()?,
    )
    .ok()?;
    append(&row, "b", "lft", &terminal.label)?;
    append(
        &row,
        "p",
        "expl",
        &format!("terminal key: {}", terminal.recipient),
    )?;
    append(
        &row,
        "p",
        "expl",
        &format!(
            "delivery: {}. access is checked when used; this does not show whether the terminal is online.",
            terminal.delivery_status
        ),
    )?;
    if terminal.delivery_status == "pending" {
        button(
            &row,
            "retry initial delivery",
            "data-terminal-retry-initial",
        )?;
    }
    for pending in &terminal.pending_additions {
        let button = button(
            &row,
            "retry pending space delivery",
            "data-terminal-retry-addition",
        )?;
        let payload = serde_json::to_string(&TerminalConnectionAddRequest {
            snapshot: String::new(),
            subjects: pending.subjects.clone(),
            operation_id: pending.operation_id.clone(),
        })
        .ok()?;
        button
            .set_attribute("data-pending-addition", &payload)
            .ok()?;
    }
    let actions = append(&row, "div", "srowd", "")?;
    button(&actions, "add spaces", "data-terminal-add-open")?;
    let revoke = button(&actions, "revoke all access", "data-terminal-revoke-all")?;
    if terminal
        .spaces
        .iter()
        .all(|g| g.targets.iter().all(|t| t.acknowledged))
    {
        revoke.set_attribute("disabled", "").ok()?;
    }
    let message = append(&row, "p", "expl", "")?;
    message
        .set_attribute("data-terminal-management-status", "")
        .ok()?;
    message.set_attribute("role", "status").ok()?;
    message.set_attribute("tabindex", "-1").ok()?;
    let additions = append(&row, "div", "terminal-additions", "")?;
    additions
        .set_attribute("data-terminal-additions", "")
        .ok()?;
    let groups = append(&row, "div", "", "")?;
    for group in &terminal.spaces {
        crate::agent_connections::append_group(&groups, group)?;
    }
    Some(())
}
fn record(target: &Element) -> Option<Element> {
    target.closest("[data-terminal-record]").ok().flatten()
}
fn id(row: &Element) -> Option<String> {
    let id = row.get_attribute("data-terminal-record")?;
    (id.len() == 64 && id.bytes().all(|c| c.is_ascii_hexdigit())).then_some(id)
}
fn busy(row: &Element, yes: bool) {
    if yes {
        let _ = row.set_attribute("data-terminal-management-busy", "");
    } else {
        let _ = row.remove_attribute("data-terminal-management-busy");
    }
}
pub(crate) fn open_add(target: &Element) {
    let Some(row) = record(target) else {
        return;
    };
    if row.has_attribute("data-terminal-management-busy") {
        return;
    }
    let Ok(Some(seat)) = row.query_selector("[data-terminal-additions]") else {
        return;
    };
    if seat.has_child_nodes() {
        return;
    }
    busy(&row, true);
    status(&row, "loading current spaces…");
    spawn_local(async move {
        let spaces = tonk_host::get_json("/api/account/terminal-links/spaces")
            .await
            .ok()
            .and_then(|s| serde_json::from_str::<TerminalLinkSpaces>(&s).ok());
        if !row.is_connected() {
            return;
        }
        busy(&row, false);
        let Some(spaces) = spaces else {
            status(&row, "spaces could not be loaded. try add spaces again.");
            return;
        };
        if row.get_attribute("data-terminal-account").as_deref() != Some(&spaces.account) {
            status(
                &row,
                "the browser account changed. refresh terminal access.",
            );
            return;
        }
        let groups: Vec<AgentConnectionSummary> = serde_json::from_str(
            &row.get_attribute("data-terminal-groups")
                .unwrap_or_default(),
        )
        .unwrap_or_default();
        let _ = seat.set_attribute("data-add-snapshot", &spaces.snapshot);
        let _ = seat.set_attribute("data-add-max", &spaces.max_spaces.to_string());
        for space in spaces.spaces {
            let existing = groups.iter().any(|g| {
                g.subject == space.subject && !matches!(g.status.as_str(), "revoked" | "expired")
            });
            let Some(label) = append(&seat, "label", "terminal-choice", "") else {
                continue;
            };
            let Some(input) =
                append(&label, "input", "", "").and_then(|n| n.dyn_into::<HtmlInputElement>().ok())
            else {
                continue;
            };
            input.set_type("checkbox");
            input.set_disabled(existing || !space.can_delegate);
            let _ = input.set_attribute("data-terminal-add-subject", &space.subject);
            let _ = append(&label, "span", "", &space.name);
            if existing {
                let _ = append(
                    &seat,
                    "p",
                    "expl",
                    "access already granted; finish revocation before granting again",
                );
            } else if !space.can_delegate {
                let _ = append(
                    &seat,
                    "p",
                    "expl",
                    space
                        .reason
                        .as_deref()
                        .unwrap_or("this account cannot delegate this space"),
                );
            }
        }
        let _ = button(&seat, "grant selected spaces", "data-terminal-add-submit");
        let _ = button(&seat, "cancel selection", "data-terminal-add-cancel");
        status(
            &row,
            "new grants last 90 days. the terminal can receive them when it next checks for additions.",
        );
    });
}
pub(crate) fn cancel_add(target: &Element) {
    let Some(row) = record(target) else {
        return;
    };
    if row.has_attribute("data-terminal-management-busy") {
        return;
    }
    if let Ok(Some(seat)) = row.query_selector("[data-terminal-additions]") {
        seat.set_text_content(None);
        let _ = seat.remove_attribute("data-add-payload");
    }
    status(
        &row,
        "selection closed. any previously sent grants remain listed after refresh.",
    );
}
pub(crate) fn submit_add(host: &HtmlElement, target: &Element) {
    let Some(row) = record(target) else {
        return;
    };
    let Some(id) = id(&row) else {
        return;
    };
    if row.has_attribute("data-terminal-management-busy") {
        return;
    }
    let Ok(Some(seat)) = row.query_selector("[data-terminal-additions]") else {
        return;
    };
    let payload = if let Some(saved) = seat.get_attribute("data-add-payload") {
        saved
    } else {
        let mut subjects = Vec::new();
        if let Ok(inputs) = seat.query_selector_all("[data-terminal-add-subject]") {
            for i in 0..inputs.length() {
                if let Some(input) = inputs
                    .item(i)
                    .and_then(|n| n.dyn_into::<HtmlInputElement>().ok())
                {
                    if input.checked() && !input.disabled() {
                        if let Some(subject) = input.get_attribute("data-terminal-add-subject") {
                            subjects.push(subject);
                        }
                    }
                }
            }
        }
        if subjects.is_empty() {
            status(&row, "select at least one available space");
            return;
        }
        let max = seat
            .get_attribute("data-add-max")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(tonk_invite::terminal::MAX_SELECTED_SPACES);
        if subjects.len() > max {
            status(
                &row,
                &format!("select at most {max} spaces for one addition. nothing has been sent."),
            );
            return;
        }
        let mut bytes = [0u8; 32];
        let Some(crypto) = web_sys::window().and_then(|w| w.crypto().ok()) else {
            return;
        };
        if crypto.get_random_values_with_u8_array(&mut bytes).is_err() {
            return;
        }
        let operation_id = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let payload = serde_json::to_string(&TerminalConnectionAddRequest {
            snapshot: seat.get_attribute("data-add-snapshot").unwrap_or_default(),
            subjects,
            operation_id,
        })
        .unwrap();
        let _ = seat.set_attribute("data-add-payload", &payload);
        if let Ok(inputs) = seat.query_selector_all("input") {
            for i in 0..inputs.length() {
                if let Some(n) = inputs.item(i).and_then(|n| n.dyn_into::<Element>().ok()) {
                    let _ = n.set_attribute("disabled", "");
                }
            }
        }
        payload
    };
    busy(&row, true);
    status(&row, "sending the complete selection…");
    let host = host.clone();
    spawn_local(async move {
        let result =
            tonk_host::post_json(&format!("/api/account/terminal-links/{id}/add"), &payload)
                .await
                .ok()
                .and_then(|s| serde_json::from_str::<TerminalConnectionAddReceipt>(&s).ok());
        if !row.is_connected() {
            return;
        }
        busy(&row, false);
        match result {
            Some(receipt) => {
                let requested: TerminalConnectionAddRequest =
                    serde_json::from_str(&payload).unwrap();
                let mut expected = requested.subjects;
                expected.sort();
                let mut actual: Vec<_> = receipt
                    .connections
                    .iter()
                    .map(|g| g.subject.clone())
                    .collect();
                actual.sort();
                if actual != expected
                    || receipt
                        .connections
                        .iter()
                        .any(|g| g.request_id.as_deref() != Some(&id))
                {
                    status(
                        &row,
                        "the receipt did not match this selection. refresh to check saved grants.",
                    );
                    return;
                }
                refresh(&host);
                announce(
                    &host,
                    "selected access was sent. the terminal can receive it when it next checks for additions.",
                );
            }
            None => status(
                &row,
                "delivery could not be confirmed. grant selected spaces retries the same decision; refresh to inspect saved grants.",
            ),
        }
    });
}
pub(crate) fn revoke_all(host: &HtmlElement, target: &Element) {
    revoke_groups(host, target, None);
}
pub(crate) fn revoke_one(host: &HtmlElement, target: &Element) {
    let Some(id) = target.get_attribute("data-connection-revoke") else {
        return;
    };
    revoke_groups(host, target, Some(vec![id]));
}
fn revoke_groups(host: &HtmlElement, target: &Element, groups: Option<Vec<String>>) {
    let Some(row) = record(target) else {
        return;
    };
    let Some(id) = id(&row) else {
        return;
    };
    if target.has_attribute("disabled") || row.has_attribute("data-terminal-management-busy") {
        return;
    }
    let payload = serde_json::json!({"groupIds": groups}).to_string();
    busy(&row, true);
    status(&row, "sending terminal grant revocations…");
    let host = host.clone();
    spawn_local(async move {
        let result = tonk_host::post_json(
            &format!("/api/account/terminal-links/{id}/revoke"),
            &payload,
        )
        .await
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<AgentConnectionSummary>>(&s).ok());
        if !row.is_connected() {
            return;
        }
        busy(&row, false);
        match result {
            Some(groups) if groups.iter().all(|g| g.request_id.as_deref() == Some(&id)) => {
                let total: usize = groups.iter().map(|g| g.targets.len()).sum();
                let acknowledged = groups
                    .iter()
                    .flat_map(|g| &g.targets)
                    .filter(|t| t.acknowledged)
                    .count();
                refresh(&host);
                announce(
                    &host,
                    &format!(
                        "revocation acknowledged for {acknowledged} of {total} grants. retry revoke all for any remaining grants. local copies and edits remain."
                    ),
                );
            }
            _ => status(
                &row,
                "revocation could not be confirmed. retry or refresh for each grant's acknowledgement. local copies and edits remain.",
            ),
        }
    });
}

pub(crate) fn retry_addition(host: &HtmlElement, target: &Element) {
    let Some(row) = record(target) else {
        return;
    };
    if row.has_attribute("data-terminal-management-busy") {
        return;
    }
    let Some(payload) = target.get_attribute("data-pending-addition") else {
        return;
    };
    let Ok(Some(seat)) = row.query_selector("[data-terminal-additions]") else {
        return;
    };
    if seat.has_child_nodes() {
        status(
            &row,
            "close the current selection before retrying a saved delivery",
        );
        return;
    }
    let _ = seat.set_attribute("data-add-payload", &payload);
    submit_add(host, target);
}
pub(crate) fn retry_initial(host: &HtmlElement, target: &Element) {
    let Some(row) = record(target) else {
        return;
    };
    let Some(id) = id(&row) else {
        return;
    };
    if row.has_attribute("data-terminal-management-busy") {
        return;
    }
    busy(&row, true);
    status(&row, "retrying the saved initial delivery…");
    let host = host.clone();
    spawn_local(async move {
        let result = tonk_host::post_json(&format!("/api/account/terminal-links/{id}/retry"), "{}")
            .await
            .ok()
            .and_then(|s| {
                serde_json::from_str::<tonk_worker_api::TerminalLinkApprovalReceipt>(&s).ok()
            });
        if !row.is_connected() {
            return;
        }
        busy(&row, false);
        if result.is_some_and(|receipt| receipt.request_id == id) {
            refresh(&host);
            announce(
                &host,
                "initial delivery acknowledged. check the terminal for completed setup.",
            );
        } else {
            status(
                &row,
                "initial delivery could not be confirmed. if the approval window expired before delivery, start a new tonk link request.",
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[wasm_bindgen_test::wasm_bindgen_test]
    fn it_keeps_pending_delivery_and_partial_revocation_visible() {
        use tonk_worker_api::{AgentConnectionTarget, TerminalPendingAddition};
        let document = web_sys::window().unwrap().document().unwrap();
        let list = document.create_element("div").unwrap();
        let request = "a".repeat(64);
        let group = AgentConnectionSummary {
            kind: Some("terminal".into()),
            request_id: Some(request.clone()),
            id: "b".repeat(64),
            repo: "repo".into(),
            subject: "did:key:space".into(),
            recipient: "did:key:terminal".into(),
            label: "workstation".into(),
            scope: "read and build".into(),
            expires_at: 1_900_000_000,
            status: "partial".into(),
            confirmed: true,
            targets: (0..6)
                .map(|n| AgentConnectionTarget {
                    cid: format!("grant-{n}"),
                    acknowledged: n < 5,
                    error: (n == 5).then(|| "unavailable".into()),
                })
                .collect(),
        };
        let terminal = TerminalConnectionSummary {
            request_id: request,
            recipient: group.recipient.clone(),
            label: group.label.clone(),
            account: "did:key:account".into(),
            delivery_status: "pending".into(),
            spaces: vec![group],
            pending_additions: vec![TerminalPendingAddition {
                operation_id: "c".repeat(64),
                delivery_id: "d".repeat(64),
                subjects: vec!["did:key:next-space".into()],
            }],
        };
        render(&list, &terminal).unwrap();
        let content = list.text_content().unwrap();
        assert!(content.contains("revocation acknowledged for 5 of 6 grants"));
        assert!(content.contains("retry revocation"));
        assert!(content.contains("this does not show whether the terminal is online"));
        assert!(
            !list
                .query_selector("[data-terminal-revoke-all]")
                .unwrap()
                .unwrap()
                .has_attribute("disabled")
        );
        let retry = list
            .query_selector("[data-terminal-retry-addition]")
            .unwrap()
            .unwrap();
        let saved: TerminalConnectionAddRequest =
            serde_json::from_str(&retry.get_attribute("data-pending-addition").unwrap()).unwrap();
        assert_eq!(
            saved.operation_id,
            terminal.pending_additions[0].operation_id
        );
        assert_eq!(saved.subjects, terminal.pending_additions[0].subjects);
        assert!(
            list.query_selector("[data-terminal-retry-initial]")
                .unwrap()
                .is_some()
        );
    }
}
