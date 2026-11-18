//! Live views as MCP Apps (`io.modelcontextprotocol/ui`). Spike.
//!
//! `tonk_show` renders the result of a notation query in an app the host draws
//! inside the conversation. The app keeps itself current by calling the
//! app-only `tonk_watch`, which long-polls: each pass reads the space through
//! the peer (which pulls first) and returns as soon as the result differs from
//! the version the app holds. MCP Apps have no server push, so this is the
//! documented way for an app to stay live.
//!
//! The page also probes what the host's sandbox allows: compiling WebAssembly
//! (needed to run Tonk's own view runtime in the app), `new Function`, and
//! fetching a loopback origin this process serves (a possible direct channel).
use std::{
    hash::{Hash, Hasher},
    time::{Duration, Instant},
};

use rmcp::model::{CallToolResult, ContentBlock, MetaObject, Resource, ResourceContents, Tool};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

use crate::owner::Peer;

const MIME: &str = "text/html;profile=mcp-app";
const TEMPLATE: &str = include_str!("view.html");
/// Entities rendered by Tonk's own guest runtime, each with its own display.
const DISPLAY: &str = include_str!("display.html");
/// How long one `tonk_watch` call may wait for a change.
const WATCH_DEFAULT: Duration = Duration::from_secs(20);
const WATCH_MAX: Duration = Duration::from_secs(25);
/// Each pass pulls from the space's service; this paces those pulls.
const WATCH_INTERVAL: Duration = Duration::from_secs(2);
/// Results larger than this are cut, and the view says so.
const MAX_MATCHES_BYTES: usize = 200_000;

pub struct Views {
    uri: String,
    html: String,
    probe: String,
    display_uri: String,
}

impl Views {
    /// Bind the loopback probe and render the page around its origin. The
    /// resource URI carries a digest of the page, so hosts that cache
    /// resources fetch a new one whenever it changes.
    pub async fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let probe = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
        tokio::spawn(serve_probe(listener));
        let html = TEMPLATE.replace(
            "__TONK_PROBE_ORIGIN__",
            &serde_json::to_string(&probe).expect("string"),
        );
        let mut digest = std::collections::hash_map::DefaultHasher::new();
        html.hash(&mut digest);
        let uri = format!("ui://tonk/view-{:016x}.html", digest.finish());
        let mut digest = std::collections::hash_map::DefaultHasher::new();
        DISPLAY.hash(&mut digest);
        let display_uri = format!("ui://tonk/display-{:016x}.html", digest.finish());
        Ok(Self {
            uri,
            html,
            probe,
            display_uri,
        })
    }

    pub fn tools(&self) -> Vec<Tool> {
        let space = json!({
            "type": "string",
            "pattern": "^did:key:[A-Za-z0-9]+$",
            "description": "Exact subject returned by tonk_list_spaces or tonk_join.",
        });
        let document = json!({ "type": "string", "minLength": 1, "maxLength": 32000 });
        [
            json!({
                "name": "tonk_show",
                "title": "Show a live Tonk view",
                "description": "Show the result of a read-only notation query as a live view the user can see in the conversation. The view keeps itself current as the space changes, including changes made elsewhere. Use it when the user wants to see or keep an eye on something in a space. Hosts without app support receive the result as text.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "space": space,
                        "document": document,
                        "title": { "type": "string", "maxLength": 200, "description": "Heading shown above the view." },
                    },
                    "required": ["space", "document"],
                    "additionalProperties": false,
                },
                "annotations": { "readOnlyHint": true, "openWorldHint": false },
                "_meta": { "ui": { "resourceUri": self.uri } },
            }),
            json!({
                "name": "tonk_display",
                "title": "Display Tonk entities",
                "description": "Render entities in the conversation with their concept's own Tonk display (the space's view for that concept), live and interactive, exactly as Tonk renders them. Find exact entity URIs and their concept names with tonk_query first. Use it when the user wants to see or work with specific things in a space.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "space": space,
                        "items": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 20,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "entity": { "type": "string", "minLength": 1, "maxLength": 500, "description": "Exact saved entity URI." },
                                    "model": { "type": "string", "minLength": 1, "maxLength": 200, "description": "Concept to display it as, such as deck/instance." },
                                    "view": { "type": "string", "maxLength": 200, "description": "A specific view concept, when the concept has several." },
                                },
                                "required": ["entity", "model"],
                                "additionalProperties": false,
                            },
                        },
                    },
                    "required": ["space", "items"],
                    "additionalProperties": false,
                },
                "annotations": { "readOnlyHint": true, "openWorldHint": false },
                "_meta": { "ui": { "resourceUri": self.display_uri } },
            }),
            json!({
                "name": "tonk_view_log",
                "title": "Read the live view log",
                "description": "Diagnostics for live views: the recent requests views made to this peer and the steps they reported, oldest first. Use it when a view stays blank or shows an error.",
                "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
                "annotations": { "readOnlyHint": true, "openWorldHint": false },
            }),
            json!({
                "name": "tonk_watch",
                "title": "Wait for a Tonk view to change",
                "description": "Used by a live Tonk view to stay current. Not for direct use.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "space": space,
                        "document": document,
                        "version": { "type": "string", "maxLength": 64 },
                        "wait_ms": { "type": "integer", "minimum": 0, "maximum": WATCH_MAX.as_millis() as u64 },
                    },
                    "required": ["space", "document", "version"],
                    "additionalProperties": false,
                },
                "annotations": { "readOnlyHint": true, "openWorldHint": false },
                "_meta": { "ui": { "visibility": ["app"] } },
            }),
        ]
        .into_iter()
        .map(|tool| serde_json::from_value(tool).expect("view tool shape"))
        .chain([crate::extension::tool()])
        .collect()
    }

    pub fn resources(&self) -> Vec<Resource> {
        vec![
            Resource::new(self.uri.clone(), "tonk-view")
                .with_title("Tonk live view")
                .with_mime_type(MIME),
            Resource::new(self.display_uri.clone(), "tonk-display")
                .with_title("Tonk display")
                .with_mime_type(MIME),
        ]
    }

    /// The display app talks to the owner's loopback worker API, so its page
    /// and permissions name that origin, asked of the owner when read.
    pub async fn read(&self, peer: &Peer, uri: &str) -> Option<Result<ResourceContents, String>> {
        if uri == self.display_uri {
            return Some(self.display(peer).await);
        }
        if uri != self.uri {
            return None;
        }
        let meta = json!({
            "ui": {
                "csp": { "connectDomains": [self.probe] },
                "prefersBorder": true,
            },
        });
        let Value::Object(meta) = meta else {
            unreachable!()
        };
        Some(Ok(ResourceContents::text(
            self.html.clone(),
            self.uri.clone(),
        )
        .with_mime_type(MIME)
        .with_meta(MetaObject::from(meta))))
    }

    async fn display(&self, peer: &Peer) -> Result<ResourceContents, String> {
        let owner = peer.call("tonk_peer_origin", json!({})).await?;
        let origin = owner["origin"]
            .as_str()
            .ok_or("The local peer did not say where it serves views.")?;
        let token = owner["token"]
            .as_str()
            .ok_or("The local peer did not give views its token.")?;
        let html = DISPLAY
            .replace(
                "__TONK_PEER_ORIGIN__",
                &serde_json::to_string(origin).expect("string"),
            )
            .replace(
                "__TONK_PEER_TOKEN__",
                &serde_json::to_string(token).expect("string"),
            );
        let Value::Object(meta) = json!({
            "ui": {
                "csp": { "connectDomains": [origin], "frameDomains": [origin] },
                "prefersBorder": true,
            },
        }) else {
            unreachable!()
        };
        Ok(ResourceContents::text(html, self.display_uri.clone())
            .with_mime_type(MIME)
            .with_meta(MetaObject::from(meta)))
    }

    /// Run a view tool, or `None` when `name` is not one.
    pub async fn call(&self, peer: &Peer, name: &str, arguments: &Value) -> Option<CallToolResult> {
        let text = |name: &str| arguments.get(name).and_then(Value::as_str).unwrap_or("");
        let (space, document) = (text("space"), text("document"));
        Some(match name {
            crate::extension::NAME => crate::extension::call(),
            "tonk_view_log" => match peer.call("tonk_view_log", json!({})).await {
                Ok(log) => {
                    let lines = log["log"].as_array().cloned().unwrap_or_default();
                    let text = lines
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("\n");
                    let mut result =
                        CallToolResult::success(vec![ContentBlock::text(if text.is_empty() {
                            "No view activity yet.".to_owned()
                        } else {
                            text
                        })]);
                    result.structured_content = Some(log);
                    result
                }
                Err(error) => CallToolResult::error(vec![ContentBlock::text(error)]),
            },
            "tonk_display" => {
                let items = arguments
                    .get("items")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                // Confirms the space is joined before the app loads it.
                match peer
                    .call("tonk_query", json!({ "space": space, "document": format!("tonk/repository:\n  this: {space}\n") }))
                    .await
                {
                    Ok(_) if items.is_empty() => {
                        CallToolResult::error(vec![ContentBlock::text("Name at least one entity.")])
                    }
                    Ok(_) => {
                        let listed = items
                            .iter()
                            .map(|item| format!("{} as {}", item["entity"].as_str().unwrap_or("?"), item["model"].as_str().unwrap_or("?")))
                            .collect::<Vec<_>>()
                            .join(", ");
                        let mut result = CallToolResult::success(vec![ContentBlock::text(format!(
                            "Displaying {listed}, rendered by Tonk."
                        ))]);
                        result.structured_content = Some(json!({ "subject": space, "items": items }));
                        result
                    }
                    Err(error) => CallToolResult::error(vec![ContentBlock::text(error)]),
                }
            }
            "tonk_show" => match snapshot(peer, space, document).await {
                Ok((version, matches, truncated)) => {
                    let title = Some(text("title")).filter(|title| !title.is_empty());
                    let summary = summarize(&matches);
                    let mut result = CallToolResult::success(vec![ContentBlock::text(format!(
                        "Showing a live view{}: {summary}. Current result: {}",
                        title
                            .map(|title| format!(" \"{title}\""))
                            .unwrap_or_default(),
                        clip(&matches.to_string(), 8_000),
                    ))]);
                    result.structured_content = Some(json!({
                        "space": space,
                        "document": document,
                        "title": title,
                        "version": version,
                        "matches": matches,
                        "truncated": truncated,
                        "peer": env!("CARGO_PKG_VERSION"),
                    }));
                    result
                }
                Err(error) => CallToolResult::error(vec![ContentBlock::text(error)]),
            },
            "tonk_watch" => {
                let held = text("version");
                let wait = arguments
                    .get("wait_ms")
                    .and_then(Value::as_u64)
                    .map(Duration::from_millis)
                    .unwrap_or(WATCH_DEFAULT)
                    .min(WATCH_MAX);
                let deadline = Instant::now() + wait;
                loop {
                    match snapshot(peer, space, document).await {
                        Ok((version, matches, truncated)) => {
                            let changed = version != held;
                            if changed || Instant::now() + WATCH_INTERVAL > deadline {
                                let mut result =
                                    CallToolResult::success(vec![ContentBlock::text(if changed {
                                        "changed"
                                    } else {
                                        "unchanged"
                                    })]);
                                result.structured_content = Some(json!({
                                    "changed": changed,
                                    "version": version,
                                    "matches": if changed { matches } else { Value::Null },
                                    "truncated": truncated,
                                }));
                                break result;
                            }
                        }
                        Err(error) => {
                            break CallToolResult::error(vec![ContentBlock::text(error)]);
                        }
                    }
                    tokio::time::sleep(WATCH_INTERVAL).await;
                }
            }
            _ => return None,
        })
    }
}

/// Read the space and reduce the worker response to its matches, versioned by
/// a digest so the view can tell whether anything changed.
async fn snapshot(
    peer: &Peer,
    space: &str,
    document: &str,
) -> Result<(String, Value, bool), String> {
    let response = peer
        .call(
            "tonk_query",
            json!({ "space": space, "document": document }),
        )
        .await?;
    let mut matches = response
        .get("matches_after")
        .cloned()
        .unwrap_or(Value::Array(Vec::new()));
    let encoded = matches.to_string();
    let mut digest = std::collections::hash_map::DefaultHasher::new();
    encoded.hash(&mut digest);
    let truncated = encoded.len() > MAX_MATCHES_BYTES;
    if truncated && let Value::Array(groups) = &mut matches {
        for group in groups.iter_mut() {
            if let Some(Value::Array(results)) = group.get_mut("results") {
                results.truncate(50);
            }
        }
    }
    Ok((format!("{:016x}", digest.finish()), matches, truncated))
}

fn summarize(matches: &Value) -> String {
    let groups = matches.as_array().map(Vec::as_slice).unwrap_or_default();
    let parts = groups
        .iter()
        .map(|group| {
            let count = group["results"].as_array().map(Vec::len).unwrap_or(0);
            format!("{count} {}", group["label"].as_str().unwrap_or("result"))
        })
        .collect::<Vec<_>>();
    if parts.is_empty() {
        "no results".into()
    } else {
        parts.join(", ")
    }
}

fn clip(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// Answer every request with a small CORS- and Private-Network-Access-enabled
/// response. It exposes nothing; it only tells the view whether the host lets
/// it reach a loopback origin at all.
async fn serve_probe(listener: TcpListener) {
    const BODY: &str = r#"{"tonk":"agent-peer","probe":true}"#;
    while let Ok((mut stream, _)) = listener.accept().await {
        tokio::spawn(async move {
            let mut request = Vec::with_capacity(1024);
            let mut chunk = [0u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < 8192 {
                match stream.read(&mut chunk).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => request.extend_from_slice(&chunk[..n]),
                }
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\n\
                 Content-Type: application/json\r\n\
                 Access-Control-Allow-Origin: *\r\n\
                 Access-Control-Allow-Methods: GET, OPTIONS\r\n\
                 Access-Control-Allow-Headers: *\r\n\
                 Access-Control-Allow-Private-Network: true\r\n\
                 Content-Length: {}\r\n\
                 Connection: close\r\n\r\n{BODY}",
                BODY.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
    }
}
