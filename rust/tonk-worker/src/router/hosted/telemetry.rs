//! What the hosted agent's `/mcp` records about each tool call: a trace
//! id, the time each step took, and how the call ended.
//!
//! The front makes a trace id for each `tools/call` and sends it to the
//! space writer in [`TRACE_HEADER`]. The writer answers the time of its
//! own steps (lock wait, pull, evaluate, push) in `Server-Timing`. The
//! front adds its own steps, answers the client both headers, and hands
//! one [`ToolEvent`] to the endpoint's sink, which the Host Worker writes
//! as one JSON line ([`log`]).
//!
//! Events are content-free: no notation, no space names, no tokens.
//! Spaces, agents and MCP sessions appear as [`short`] hashes, enough to
//! group events by them and nothing more.

use axum::http::{HeaderMap, HeaderValue};
use dialog_varsig::Did;
use serde::Serialize;
use tonk_common::diagnostics;

/// The header that carries a tool call's trace id, from the front to the
/// space writer and back to the client.
pub const TRACE_HEADER: &str = "x-tonk-trace";

/// The `Server-Timing` header.
pub const SERVER_TIMING: &str = "server-timing";

/// A fresh trace id: 16 hex digits.
pub fn trace() -> String {
    format!("{:016x}", rand::random::<u64>())
}

/// The trace id `headers` carry, if it is one [`trace`] could have made.
pub fn trace_of(headers: &HeaderMap) -> Option<String> {
    headers
        .get(TRACE_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|trace| trace.len() == 16 && trace.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_owned)
}

/// A short, stable stand-in for `value`: the first 8 hex digits of its
/// BLAKE3 hash.
pub fn short(value: &str) -> String {
    blake3::hash(value.as_bytes()).to_hex()[..8].to_owned()
}

/// [`short`] for a DID.
pub fn short_did(did: &Did) -> String {
    short(did.as_ref())
}

/// The time each step of a request took, in the order they ran.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Steps {
    steps: Vec<(String, f64)>,
    /// When the step in progress began ([`diagnostics::now_ms`]).
    since: f64,
}

impl Steps {
    /// Steps timed from now.
    pub fn start() -> Self {
        Self {
            steps: Vec::new(),
            since: diagnostics::now_ms(),
        }
    }

    /// End the step in progress as `name`, and begin the next.
    pub fn lap(&mut self, name: &str) {
        let now = diagnostics::now_ms();
        self.steps.push((name.to_owned(), now - self.since));
        self.since = now;
    }

    /// Begin the next step now, without recording the time since the last
    /// one.
    pub fn skip(&mut self) {
        self.since = diagnostics::now_ms();
    }

    /// Record `name` as taking `ms`.
    pub fn push(&mut self, name: &str, ms: f64) {
        self.steps.push((name.to_owned(), ms));
    }

    /// Record the steps of `other` after these.
    pub fn extend(&mut self, other: Steps) {
        self.steps.extend(other.steps);
    }

    /// Record the steps of `other` after these, each named
    /// `{within}.{name}`: the parts of a step `within` that another party
    /// timed, such as the writer's pull inside the front's `writer` step.
    pub fn extend_within(&mut self, within: &str, other: Steps) {
        self.steps.extend(
            other
                .steps
                .into_iter()
                .map(|(name, ms)| (format!("{within}.{name}"), ms)),
        );
    }

    /// The steps, in order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, f64)> {
        self.steps.iter().map(|(name, ms)| (name.as_str(), *ms))
    }

    /// Whether no step is recorded.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// The steps as a `Server-Timing` value: `pull;dur=12.3, eval;dur=4.0`.
    pub fn header(&self) -> String {
        self.iter()
            .map(|(name, ms)| format!("{name};dur={ms:.1}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The steps a `Server-Timing` value names, in order. Entries without
    /// a duration, or with a name that is not a token, are skipped.
    pub fn parse(header: &str) -> Self {
        let steps = header
            .split(',')
            .filter_map(|entry| {
                let mut parts = entry.trim().split(';');
                let name = parts.next()?.trim();
                if name.is_empty() || !name.bytes().all(is_token) {
                    return None;
                }
                let ms = parts
                    .filter_map(|part| part.trim().strip_prefix("dur="))
                    .find_map(|ms| ms.parse::<f64>().ok())?;
                Some((name.to_owned(), ms))
            })
            .collect();
        Self {
            steps,
            since: diagnostics::now_ms(),
        }
    }

    /// The steps `headers` carry in `Server-Timing`.
    pub fn of(headers: &HeaderMap) -> Self {
        headers
            .get_all(SERVER_TIMING)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .map(Self::parse)
            .fold(Self::default(), |mut all, steps| {
                all.extend(steps);
                all
            })
    }

    /// Set `headers`' `Server-Timing` to these steps, when there are any.
    pub fn insert(&self, headers: &mut HeaderMap) {
        if self.is_empty() {
            return;
        }
        if let Ok(value) = HeaderValue::from_str(&self.header()) {
            headers.insert(SERVER_TIMING, value);
        }
    }
}

fn is_token(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

/// How a tool call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// It answered without `isError`.
    Ok,
    /// It was refused for want of access: no grant, a read-only grant for
    /// a write, or a lapsed grant.
    Refused,
    /// The agent's own request was rejected: missing or malformed
    /// arguments, or notation the space answered as malformed. Not a
    /// failure of the service.
    Invalid,
    /// It failed: the writer, the sync or the evaluation answered an error.
    Error,
}

impl Outcome {
    /// The diagnostics counter for this outcome.
    pub fn counter(self) -> &'static str {
        match self {
            Outcome::Ok => diagnostics::names::MCP_TOOL_OK,
            Outcome::Refused => diagnostics::names::MCP_TOOL_REFUSED,
            Outcome::Invalid => diagnostics::names::MCP_TOOL_INVALID,
            Outcome::Error => diagnostics::names::MCP_TOOL_ERROR,
        }
    }
}

/// One tool call, as the front saw it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolEvent {
    /// Always `mcp.tool`.
    pub event: &'static str,
    /// The call's trace id ([`trace`]).
    pub trace: String,
    /// The tool, or `other` for a name this endpoint does not serve.
    pub tool: String,
    /// How the call ended.
    pub outcome: Outcome,
    /// Why it was refused, rejected or failed: a reason this endpoint
    /// names, or the writer's error code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// From the request arriving to the answer, in milliseconds.
    pub ms: f64,
    /// The time each step took, in milliseconds.
    pub steps: serde_json::Map<String, serde_json::Value>,
    /// The space acted on, as a [`short`] hash.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
    /// The agent space, as a [`short`] hash.
    pub agent: String,
    /// The MCP session, as a [`short`] hash.
    pub session: String,
}

impl ToolEvent {
    /// The time recorded for `step`, if any.
    pub fn step(&self, step: &str) -> Option<f64> {
        self.steps.get(step).and_then(serde_json::Value::as_f64)
    }
}

/// Steps as an event's `steps` map. A step that ran twice (two pulls)
/// is summed.
pub fn step_map(steps: &Steps) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    for (name, ms) in steps.iter() {
        let sum = map
            .get(name)
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0)
            + ms;
        map.insert(name.to_owned(), serde_json::json!(round(sum)));
    }
    map
}

/// `ms` to a tenth of a millisecond.
pub fn round(ms: f64) -> f64 {
    (ms * 10.0).round() / 10.0
}

/// Where an endpoint sends its events.
pub type Sink<'a> = &'a dyn Fn(&ToolEvent);

/// The Host Worker's sink: count the outcome and time in this realm's
/// diagnostics, and write the event as one JSON line, which Workers Logs
/// keeps and can query by field.
pub fn log(event: &ToolEvent) {
    diagnostics::count(event.outcome.counter());
    diagnostics::record(diagnostics::names::MCP_TOOL_CALL, event.ms);
    if let Ok(line) = serde_json::to_string(event) {
        tonk_common::log!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn steps_read_back_from_their_header() {
        let mut steps = Steps::default();
        steps.push("lock", 0.04);
        steps.push("pull", 120.0);
        steps.push("eval", 3.24);
        let header = steps.header();
        assert_eq!(header, "lock;dur=0.0, pull;dur=120.0, eval;dur=3.2");
        let parsed: Vec<_> = Steps::parse(&header)
            .iter()
            .map(|(name, ms)| (name.to_owned(), ms))
            .collect();
        assert_eq!(
            parsed,
            vec![
                ("lock".to_owned(), 0.0),
                ("pull".to_owned(), 120.0),
                ("eval".to_owned(), 3.2)
            ]
        );
    }

    #[dialog_common::test]
    fn entries_without_a_duration_or_a_token_name_are_skipped() {
        let parsed: Vec<_> = Steps::parse("cache;desc=hit, pull;dur=5, bad name;dur=1, ;dur=2")
            .iter()
            .map(|(name, ms)| (name.to_owned(), ms))
            .collect();
        assert_eq!(parsed, vec![("pull".to_owned(), 5.0)]);
    }

    #[dialog_common::test]
    fn a_step_that_ran_twice_is_summed() {
        let mut steps = Steps::default();
        steps.push("pull", 1.0);
        steps.push("eval", 2.0);
        steps.push("pull", 3.0);
        let map = step_map(&steps);
        assert_eq!(map["pull"], serde_json::json!(4.0));
        assert_eq!(map["eval"], serde_json::json!(2.0));
    }

    #[dialog_common::test]
    fn only_a_trace_id_of_the_minted_shape_is_taken() {
        let mut headers = HeaderMap::new();
        headers.insert(TRACE_HEADER, HeaderValue::from_static("0123456789abcdef"));
        assert_eq!(trace_of(&headers).as_deref(), Some("0123456789abcdef"));
        headers.insert(TRACE_HEADER, HeaderValue::from_static("not a trace"));
        assert_eq!(trace_of(&headers), None);
        assert_eq!(trace().len(), 16);
    }

    #[dialog_common::test]
    fn an_event_names_no_content() {
        let event = ToolEvent {
            event: "mcp.tool",
            trace: trace(),
            tool: "tonk_query".into(),
            outcome: Outcome::Refused,
            kind: Some("read_only".into()),
            ms: 1.5,
            steps: step_map(&Steps::default()),
            space: Some(short("did:key:z6Mkspace")),
            agent: short("did:key:z6Mkagent"),
            session: short("session"),
        };
        let line = serde_json::to_value(&event).unwrap();
        assert_eq!(line["outcome"], "refused");
        assert_eq!(line["space"].as_str().unwrap().len(), 8);
        assert!(!line.to_string().contains("did:key"), "{line}");
    }
}
