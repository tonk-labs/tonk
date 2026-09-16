//! Browser selection for a signed CLI-owned terminal request.
//! The worker revalidates the account, snapshot and complete selection.

use tonk_invite::terminal::LinkRequest;
use tonk_worker_api::{
    TerminalLinkApprovalReceipt, TerminalLinkApproveRequest, TerminalLinkSpaces,
};
use wasm_bindgen::JsCast as _;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, HtmlElement, HtmlInputElement};

fn node(host: &HtmlElement, selector: &str) -> Option<Element> {
    host.query_selector(selector).ok().flatten()
}

fn text(host: &HtmlElement, selector: &str, value: &str) {
    if let Some(node) = node(host, selector) {
        node.set_text_content(Some(value));
    }
}

fn status(host: &HtmlElement, value: &str) {
    text(host, "[data-terminal-status]", value);
}

fn enabled(host: &HtmlElement, selector: &str, yes: bool) {
    if let Some(node) = node(host, selector) {
        if yes {
            let _ = node.remove_attribute("disabled");
        } else {
            let _ = node.set_attribute("disabled", "");
        }
    }
}

fn generation(host: &HtmlElement) -> String {
    let next = host
        .get_attribute("data-terminal-generation")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
        .wrapping_add(1)
        .to_string();
    let _ = host.set_attribute("data-terminal-generation", &next);
    next
}

fn current(host: &HtmlElement, token: &str) -> bool {
    host.is_connected()
        && host.has_attribute("data-terminal-url")
        && host.get_attribute("data-terminal-generation").as_deref() == Some(token)
}

fn now() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}

pub(crate) fn is_request(path: &str, hash: &str) -> bool {
    path == "/settings/link" && hash.starts_with("#tonk-terminal-")
}

pub(crate) fn leave(host: &HtmlElement) {
    if host.has_attribute("data-terminal-url") {
        generation(host);
        for name in [
            "url",
            "ready",
            "loading",
            "busy",
            "complete",
            "uncertain",
            "snapshot",
            "request",
            "request-id",
        ] {
            let _ = host.remove_attribute(&format!("data-terminal-{name}"));
        }
    }
}

pub(crate) fn open(host: &HtmlElement, url: &str) {
    if host.get_attribute("data-terminal-url").as_deref() == Some(url) {
        return;
    }
    leave(host);
    let _ = host.set_attribute("data-terminal-url", url);
    reload(host);
}

pub(crate) fn refresh_if_unready(host: &HtmlElement) {
    if host.has_attribute("data-terminal-url")
        && !host.has_attribute("data-terminal-ready")
        && !host.has_attribute("data-terminal-loading")
        && !host.has_attribute("data-terminal-complete")
    {
        reload(host);
    }
}

pub(crate) fn reload(host: &HtmlElement) {
    let Some(url) = host.get_attribute("data-terminal-url") else {
        return;
    };
    if host.has_attribute("data-terminal-busy") {
        return;
    }
    let token = generation(host);
    for name in [
        "ready",
        "complete",
        "uncertain",
        "snapshot",
        "request",
        "request-id",
    ] {
        let _ = host.remove_attribute(&format!("data-terminal-{name}"));
    }
    let _ = host.set_attribute("data-terminal-loading", "");
    text(host, "[data-terminal-name]", "this terminal");
    enabled(host, "[data-terminal-refresh]", true);
    for selector in [
        "[data-terminal-all]",
        "[data-terminal-approve]",
        "[data-terminal-decline]",
    ] {
        enabled(host, selector, false);
    }
    if let Some(list) = node(host, "[data-terminal-spaces]") {
        list.set_text_content(None);
    }
    status(host, "loading spaces…");
    let host = host.clone();
    spawn_local(async move {
        let request = LinkRequest::from_url(&url, now()).await;
        if !current(&host, &token) {
            return;
        }
        let Ok(request) = request else {
            let _ = host.remove_attribute("data-terminal-loading");
            status(
                &host,
                "this request is invalid or expired. run tonk link again in the terminal.",
            );
            return;
        };
        text(&host, "[data-terminal-name]", request.label());
        let result = tonk_host::get_json("/api/account/terminal-links/spaces")
            .await
            .ok()
            .and_then(|body| serde_json::from_str::<TerminalLinkSpaces>(&body).ok());
        if !current(&host, &token) {
            return;
        }
        let _ = host.remove_attribute("data-terminal-loading");
        let Some(spaces) = result else {
            status(&host, "unable to load spaces. sign in, then refresh.");
            return;
        };
        if request
            .expected_account()
            .is_some_and(|expected| expected.as_str() != spaces.account)
        {
            status(
                &host,
                "switch to the account you used to start this request, then refresh.",
            );
            return;
        }
        if spaces.grant_lifetime_seconds != 90 * 24 * 60 * 60 {
            status(&host, "reload this page, then run tonk link again.");
            return;
        }
        let encoded: String = request
            .bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let _ = host.set_attribute("data-terminal-request", &encoded);
        let _ = host.set_attribute("data-terminal-request-id", &request.id());
        let _ = host.set_attribute("data-terminal-snapshot", &spaces.snapshot);
        let _ = host.set_attribute("data-terminal-deadline", &request.deadline().to_string());
        let _ = host.set_attribute("data-terminal-ready", "");
        let _ = host.set_attribute("data-terminal-max-spaces", &spaces.max_spaces.to_string());
        if let Some(list) = node(&host, "[data-terminal-spaces]") {
            for space in &spaces.spaces {
                let _ = append_space(&list, space);
            }
        }
        enabled(&host, "[data-terminal-decline]", true);
        selection_changed(&host, false);
        let remaining = request.deadline().saturating_sub(now());
        gloo_timers::future::TimeoutFuture::new((remaining.min(600) * 1000) as u32).await;
        if current(&host, &token)
            && !host.has_attribute("data-terminal-complete")
            && !host.has_attribute("data-terminal-busy")
        {
            enabled(&host, "[data-terminal-approve]", false);
            enabled(&host, "[data-terminal-decline]", false);
            status(
                &host,
                "this approval request expired. run tonk link again in the terminal.",
            );
        }
    });
}

fn append_space(list: &Element, space: &tonk_worker_api::TerminalLinkSpace) -> Option<()> {
    let document = list.owner_document()?;
    let row = document.create_element("label").ok()?;
    row.set_class_name("terminal-space terminal-choice");
    let input: HtmlInputElement = document.create_element("input").ok()?.dyn_into().ok()?;
    input.set_type("checkbox");
    input.set_disabled(!space.can_delegate);
    input
        .set_attribute("data-terminal-subject", &space.subject)
        .ok()?;
    input
        .set_attribute(
            "data-terminal-eligible",
            if space.can_delegate { "true" } else { "false" },
        )
        .ok()?;
    row.append_child(&input).ok()?;
    let copy = document.create_element("span").ok()?;
    copy.set_class_name("terminal-space-copy");
    let name = document.create_element("span").ok()?;
    name.set_text_content(Some(&space.name));
    copy.append_child(&name).ok()?;
    if !space.can_delegate {
        let reason = document.create_element("span").ok()?;
        reason.set_class_name("expl");
        reason.set_text_content(Some("ask the space owner for access."));
        copy.append_child(&reason).ok()?;
    }
    row.append_child(&copy).ok()?;
    list.append_child(&row).ok()?;
    Some(())
}

fn choices(host: &HtmlElement) -> Vec<HtmlInputElement> {
    let Ok(nodes) = host.query_selector_all("[data-terminal-subject]") else {
        return vec![];
    };
    (0..nodes.length())
        .filter_map(|index| nodes.item(index)?.dyn_into().ok())
        .collect()
}

pub(crate) fn selection_changed(host: &HtmlElement, all_changed: bool) {
    if host.has_attribute("data-terminal-busy")
        || host.has_attribute("data-terminal-complete")
        || host.has_attribute("data-terminal-uncertain")
    {
        return;
    }
    let all =
        node(host, "[data-terminal-all]").and_then(|node| node.dyn_into::<HtmlInputElement>().ok());
    let choices = choices(host);
    let eligible: Vec<_> = choices
        .iter()
        .filter(|input| input.get_attribute("data-terminal-eligible").as_deref() == Some("true"))
        .collect();
    if all_changed {
        for input in &eligible {
            input.set_checked(all.as_ref().is_some_and(|all| all.checked()));
        }
    }
    let selected = eligible.iter().filter(|input| input.checked()).count();
    if let Some(all) = all {
        all.set_checked(!eligible.is_empty() && selected == eligible.len());
        all.set_indeterminate(selected > 0 && selected < eligible.len());
        all.set_disabled(eligible.is_empty());
    }
    let active = host.has_attribute("data-terminal-ready")
        && host
            .get_attribute("data-terminal-deadline")
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|deadline| deadline > now());
    let max = host
        .get_attribute("data-terminal-max-spaces")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(tonk_invite::terminal::MAX_SELECTED_SPACES);
    enabled(
        host,
        "[data-terminal-approve]",
        active && selected > 0 && selected <= max,
    );
    if selected > max {
        status(host, &format!("choose up to {max} spaces at a time."));
        return;
    }
    let action = match selected {
        0 => "link spaces".to_owned(),
        1 => "link 1 space".to_owned(),
        count => format!("link {count} spaces"),
    };
    text(host, "[data-terminal-approve]", &action);
    status(
        host,
        if choices.is_empty() {
            "no spaces to link yet. create a space, then refresh."
        } else if eligible.is_empty() {
            "ask a space owner for access, then refresh."
        } else {
            ""
        },
    );
}

pub(crate) fn submit(host: &HtmlElement, decline: bool) {
    let selector = if decline {
        "[data-terminal-decline]"
    } else {
        "[data-terminal-approve]"
    };
    if node(host, selector).is_none_or(|node| node.has_attribute("disabled"))
        || host.has_attribute("data-terminal-busy")
    {
        return;
    }
    let Some(request) = host.get_attribute("data-terminal-request") else {
        return;
    };
    let Some(snapshot) = host.get_attribute("data-terminal-snapshot") else {
        return;
    };
    let Some(request_id) = host.get_attribute("data-terminal-request-id") else {
        return;
    };
    let subjects: Vec<String> = choices(host)
        .iter()
        .filter(|input| {
            input.checked()
                && input.get_attribute("data-terminal-eligible").as_deref() == Some("true")
        })
        .filter_map(|input| input.get_attribute("data-terminal-subject"))
        .collect();
    if !decline && subjects.is_empty() {
        return;
    }
    let mut expected_subjects = if decline { vec![] } else { subjects.clone() };
    expected_subjects.sort();
    let body = if decline {
        serde_json::json!({"request":request,"snapshot":snapshot}).to_string()
    } else {
        serde_json::to_string(&TerminalLinkApproveRequest {
            request,
            snapshot,
            subjects,
        })
        .unwrap_or_default()
    };
    let _ = host.set_attribute("data-terminal-busy", "");
    for input in choices(host) {
        input.set_disabled(true);
    }
    for selector in [
        "[data-terminal-all]",
        "[data-terminal-refresh]",
        "[data-terminal-approve]",
        "[data-terminal-decline]",
    ] {
        enabled(host, selector, false);
    }
    status(
        host,
        if decline {
            "cancelling…"
        } else {
            "linking spaces…"
        },
    );
    let token = host
        .get_attribute("data-terminal-generation")
        .unwrap_or_default();
    let host = host.clone();
    spawn_local(async move {
        let endpoint = if decline {
            "/api/account/terminal-links/decline"
        } else {
            "/api/account/terminal-links/approve"
        };
        let result = tonk_host::post_json(endpoint, &body)
            .await
            .ok()
            .and_then(|body| serde_json::from_str::<TerminalLinkApprovalReceipt>(&body).ok());
        if !current(&host, &token) {
            return;
        }
        let _ = host.remove_attribute("data-terminal-busy");
        match result {
            Some(receipt)
                if receipt.request_id == request_id && {
                    let mut delivered: Vec<_> = receipt
                        .connections
                        .iter()
                        .map(|connection| connection.subject.clone())
                        .collect();
                    delivered.sort();
                    delivered == expected_subjects
                } =>
            {
                let _ = host.set_attribute("data-terminal-complete", "");
                status(
                    &host,
                    if decline {
                        "request cancelled. you can close this tab."
                    } else {
                        "spaces approved. return to your terminal to finish."
                    },
                );
            }
            _ => {
                let _ = host.set_attribute("data-terminal-uncertain", "");
                status(
                    &host,
                    "unable to confirm. check your terminal, or try again.",
                );
                enabled(&host, selector, true);
                enabled(&host, "[data-terminal-refresh]", true);
                text(
                    &host,
                    selector,
                    if decline { "retry cancel" } else { "try again" },
                );
            }
        }
        if let Some(status) = node(&host, "[data-terminal-status]")
            .and_then(|node| node.dyn_into::<HtmlElement>().ok())
        {
            let _ = status.focus();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    #[wasm_bindgen_test]
    fn it_selects_only_available_snapshot_spaces() {
        let document = web_sys::window().unwrap().document().unwrap();
        let host: HtmlElement = document.create_element("div").unwrap().dyn_into().unwrap();
        host.set_inner_html(include_str!("ui_account_settings.html"));
        document.body().unwrap().append_child(&host).unwrap();
        host.set_attribute("data-terminal-ready", "").unwrap();
        host.set_attribute("data-terminal-deadline", &(now() + 60).to_string())
            .unwrap();
        let list = node(&host, "[data-terminal-spaces]").unwrap();
        for (name, subject, allowed) in [
            ("Project notes", "did:key:owned", true),
            ("Team space", "did:key:shared", true),
            ("Read only", "did:key:read-only", false),
        ] {
            append_space(
                &list,
                &tonk_worker_api::TerminalLinkSpace {
                    repo: subject.into(),
                    subject: subject.into(),
                    name: name.into(),
                    can_delegate: allowed,
                    reason: (!allowed).then(|| "read-only authority".into()),
                },
            )
            .unwrap();
        }
        assert!(!list.text_content().unwrap().contains("did:key:"));
        assert_eq!(
            list.query_selector_all("label.terminal-space")
                .unwrap()
                .length(),
            3
        );
        assert!(
            list.text_content()
                .unwrap()
                .contains("ask the space owner for access.")
        );
        let all: HtmlInputElement = node(&host, "[data-terminal-all]")
            .unwrap()
            .dyn_into()
            .unwrap();
        all.set_checked(true);
        selection_changed(&host, true);
        let inputs = choices(&host);
        assert!(inputs[0].checked() && inputs[1].checked());
        assert!(!inputs[2].checked() && inputs[2].disabled());
        assert_eq!(
            node(&host, "[data-terminal-approve]")
                .unwrap()
                .text_content()
                .unwrap(),
            "link 2 spaces"
        );
        assert!(
            !node(&host, "[data-terminal-approve]")
                .unwrap()
                .has_attribute("disabled")
        );
        inputs[0].set_checked(false);
        selection_changed(&host, false);
        assert!(all.indeterminate());
        assert_eq!(
            node(&host, "[data-terminal-approve]")
                .unwrap()
                .text_content()
                .unwrap(),
            "link 1 space"
        );
        inputs[1].set_checked(false);
        selection_changed(&host, false);
        assert!(
            node(&host, "[data-terminal-approve]")
                .unwrap()
                .has_attribute("disabled")
        );
        assert!(
            node(&host, "[data-terminal-status]")
                .unwrap()
                .text_content()
                .unwrap()
                .is_empty()
        );
        host.remove();
    }
}
