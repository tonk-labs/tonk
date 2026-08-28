//! Pure rendering for the inspector's read-only branch diagnostics.
//!
//! The sealed guest deliberately does not link the worker or repository
//! engine. These small serde mirrors decode only the fields the panel renders;
//! unknown fields remain forward-compatible and missing optional fields degrade
//! to explicit local-only / no-commit states.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

/// Current state of the explicit remote-status probe.
#[derive(Clone, Copy, Debug, Default)]
pub enum Probe<'a> {
    /// The remote has not been contacted.
    #[default]
    Idle,
    /// A probe is in flight.
    Loading,
    /// Raw JSON returned by `/sync/status`.
    Response(&'a str),
    /// The probe failed without replacing the last local metadata.
    Failure(&'a str),
}

#[derive(Debug, Deserialize)]
struct RepositoryInfo {
    name: String,
    label: String,
    subject: String,
    operator: String,
    profile: String,
    #[serde(default)]
    branch: BTreeMap<String, BranchInfo>,
    #[serde(default)]
    remote: BTreeMap<String, RemoteInfo>,
}

#[derive(Debug, Default, Deserialize)]
struct BranchInfo {
    #[serde(default)]
    upstream: Option<UpstreamInfo>,
    #[serde(default)]
    revision: Option<Revision>,
}

#[derive(Debug, Deserialize)]
struct UpstreamInfo {
    remote: String,
    branch: String,
}

#[derive(Debug, Deserialize)]
struct RemoteInfo {
    address: Value,
    #[serde(default)]
    subject: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Revision {
    #[serde(default)]
    tree: Value,
}

#[derive(Debug, Deserialize)]
struct SyncStatus {
    state: String,
    #[serde(default)]
    local: Option<Revision>,
    #[serde(default)]
    remote: Option<Revision>,
}

/// Render the initial local-metadata loading state.
pub fn render_loading(repo: &str, branch: &str) -> String {
    format!(
        "{}<div class=\"inspector-debug__notice\" role=\"status\">reading local repository metadata…</div>{}",
        disclosure_start(branch, repo, "reading metadata…", false, false),
        disclosure_end(),
    )
}

/// Render a local-metadata failure without affecting the notebook below it.
pub fn render_failure(repo: &str, branch: &str, message: &str) -> String {
    format!(
        "{}<div class=\"inspector-debug__notice is-error\" role=\"status\">{}</div>{}",
        disclosure_start(branch, repo, "metadata unavailable", true, false),
        esc(message),
        disclosure_end(),
    )
}

/// Decode repository metadata and render the complete diagnostics panel body.
pub fn render_repository(
    _repo: &str,
    branch: &str,
    body: &str,
    probe: Probe<'_>,
) -> Result<String, String> {
    let info: RepositoryInfo = serde_json::from_str(body)
        .map_err(|error| format!("repository metadata decode: {error}"))?;
    let branch_info = info.branch.get(branch);
    let revision = branch_info
        .and_then(|branch| branch.revision.as_ref())
        .as_ref()
        .and_then(|revision| tree_display(&revision.tree))
        .unwrap_or_else(|| "no commits".to_owned());
    let upstream = branch_info.and_then(|branch| branch.upstream.as_ref());
    let remote = upstream.and_then(|upstream| info.remote.get(&upstream.remote));

    let summary_status = if upstream.is_some() {
        "remote configured"
    } else {
        "local only"
    };
    let mut html = disclosure_start(branch, &revision, summary_status, true, upstream.is_some());
    html.push_str("<dl class=\"inspector-debug__rows\">");
    html.push_str(&row("space", &info.label, false));
    html.push_str(&row("route", &info.name, true));
    html.push_str(&row("branch", branch, true));
    html.push_str(&row("revision", &revision, revision != "no commits"));

    match upstream {
        Some(upstream) => {
            html.push_str(&row(
                "upstream",
                &format!("{}/{}", upstream.remote, upstream.branch),
                true,
            ));
            match remote {
                Some(remote) => {
                    html.push_str(&row("remote", &remote_address(&remote.address), true));
                    html.push_str(&row(
                        "remote subject",
                        remote.subject.as_deref().unwrap_or(&info.subject),
                        true,
                    ));
                }
                None => html.push_str(&row("remote", "configuration missing", false)),
            }
        }
        None => {
            html.push_str(&row("upstream", "none — local only", false));
            html.push_str(&row("remote", "not configured", false));
        }
    }

    html.push_str(&row("repository", &info.subject, true));
    html.push_str(&row("profile", &info.profile, true));
    html.push_str(&row("operator", &info.operator, true));
    if upstream.is_some() {
        html.push_str(&render_probe(probe)?);
    }
    html.push_str("</dl>");
    html.push_str(disclosure_end());
    Ok(html)
}

fn render_probe(probe: Probe<'_>) -> Result<String, String> {
    match probe {
        Probe::Idle => Ok(row("sync", "not probed", false)),
        Probe::Loading => Ok(row("sync", "probing remote…", false)),
        Probe::Failure(message) => Ok(format!(
            "<div class=\"inspector-debug__probe-error\"><dt>sync</dt><dd>{}</dd></div>",
            esc(message),
        )),
        Probe::Response(body) => {
            let status: SyncStatus = serde_json::from_str(body)
                .map_err(|error| format!("sync status decode: {error}"))?;
            let mut html = row("sync", &status.state, false);
            let local = status
                .local
                .as_ref()
                .and_then(|revision| tree_display(&revision.tree))
                .unwrap_or_else(|| "no commits".to_owned());
            let remote = status
                .remote
                .as_ref()
                .and_then(|revision| tree_display(&revision.tree))
                .unwrap_or_else(|| "no commits".to_owned());
            html.push_str(&row("local head", &local, local != "no commits"));
            html.push_str(&row("remote head", &remote, remote != "no commits"));
            Ok(html)
        }
    }
}

fn disclosure_start(
    branch: &str,
    value: &str,
    status: &str,
    can_refresh: bool,
    can_probe: bool,
) -> String {
    let refresh = if can_refresh {
        "<button type=\"button\" data-debug-action=\"refresh\">refresh</button>"
    } else {
        ""
    };
    let probe = if can_probe {
        "<button type=\"button\" data-debug-action=\"probe\">probe remote</button>"
    } else {
        ""
    };
    format!(
        "<details class=\"inspector-debug__disclosure\">\
           <summary class=\"inspector-debug__summary\">\
             <span>branch diagnostics</span><strong>{}</strong>\
             <code title=\"{}\">{}</code><small>{}</small>\
           </summary>\
           <div class=\"inspector-debug__body\">\
             <div class=\"inspector-debug__actions\">{refresh}{probe}</div>",
        esc(branch),
        esc_attr(value),
        esc(value),
        esc(status),
    )
}

fn disclosure_end() -> &'static str {
    "</div></details>"
}

fn row(label: &str, value: &str, copyable: bool) -> String {
    let copy = if copyable {
        format!(
            "<button type=\"button\" class=\"inspector-debug__copy\" data-copy-value=\"{}\" aria-live=\"polite\" aria-label=\"copy {}\">copy</button>",
            esc_attr(value),
            esc_attr(label),
        )
    } else {
        String::new()
    };
    format!(
        "<div class=\"inspector-debug__row\"><dt>{}</dt><dd><code>{}</code>{copy}</dd></div>",
        esc(label),
        esc(value),
    )
}

fn remote_address(address: &Value) -> String {
    address
        .get("Ucan")
        .and_then(|ucan| ucan.get("endpoint"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| serde_json::to_string(address).unwrap_or_else(|_| "unknown".to_owned()))
}

fn tree_display(tree: &Value) -> Option<String> {
    match tree {
        Value::String(value) if !value.is_empty() => Some(value.clone()),
        Value::Array(bytes) => {
            let raw: Option<Vec<u8>> = bytes
                .iter()
                .map(|byte| byte.as_u64().and_then(|value| u8::try_from(value).ok()))
                .collect();
            raw.filter(|bytes| !bytes.is_empty())
                .map(|bytes| format!("#{}", bs58::encode(bytes).into_string()))
        }
        _ => None,
    }
}

fn esc(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn esc_attr(value: &str) -> String {
    esc(value)
}
