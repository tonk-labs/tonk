//! The MCP surface. Tool definitions come from the shared catalog in
//! `mcp/tools.json`, so every Tonk host describes the same tools the same way;
//! guides are the CLI's canonical manuals. Live views (MCP Apps) are offered
//! beside them by [`crate::views`].
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Icon,
        Implementation, ListResourcesResult, ListToolsResult, PaginatedRequestParams,
        ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
        ResourceContents, ServerCapabilities, ServerConfig, Tool,
    },
    service::RequestContext,
};
use serde_json::{Value, json};
use tonk_evaluator::{evaluate::QueryMatchBlock, notation::render_matches};

use std::sync::Arc;

use crate::{guide, owner::Peer, views::Views};

const CATALOG: &str = include_str!("../../../mcp/tools.json");
/// Tools this host offers, in catalog order.
const TOOLS: &[&str] = &[
    "tonk_list_spaces",
    "tonk_join",
    "tonk_query",
    "tonk_evaluate",
    "tonk_guide",
];
/// Tools that act on one joined space take its subject.
const SCOPED: &[&str] = &["tonk_query", "tonk_evaluate"];

#[derive(Clone)]
pub struct Server {
    peer: Peer,
    tools: Vec<Tool>,
    views: Arc<Views>,
}

impl Server {
    pub fn new(peer: Peer, views: Arc<Views>) -> Self {
        let catalog: Value = serde_json::from_str(CATALOG).expect("valid tool catalog");
        let tools = catalog["tools"]
            .as_array()
            .expect("catalog lists tools")
            .iter()
            .filter(|tool| TOOLS.contains(&tool["name"].as_str().unwrap_or_default()))
            .map(|tool| {
                let mut tool = tool.clone();
                // The shared catalog describes the hosted connector, which
                // returns the worker's JSON; the peer answers in notation.
                if let Some(description) = tool["description"].as_str() {
                    let description = description.replace(
                        "Returns the unchanged worker evaluation response.",
                        NOTATION_RESULT,
                    );
                    tool["description"] = json!(description);
                }
                if SCOPED.contains(&tool["name"].as_str().unwrap_or_default()) {
                    tool["inputSchema"]["properties"]["space"] = json!({
                        "type": "string",
                        "pattern": "^did:key:[A-Za-z0-9]+$",
                        "description": "Exact subject returned by tonk_list_spaces or tonk_join.",
                    });
                    tool["inputSchema"]["required"]
                        .as_array_mut()
                        .expect("catalog required list")
                        .push(json!("space"));
                }
                serde_json::from_value(tool).expect("catalog tool shape")
            })
            .collect::<Vec<Tool>>();
        assert_eq!(tools.len(), TOOLS.len(), "catalog misses a peer tool");
        let mut tools = tools;
        tools.extend(views.tools());
        Self { peer, tools, views }
    }
}

const NOTATION_RESULT: &str = "Returns the matches as Tonk notation, one `<concept>:` block per match with `this:` and its fields, the same form `tonk eval` prints.";

/// A query or evaluation answered as notation, the form agents read and write:
/// each match as a `<concept>:` block, and for a write the committed claims and
/// entities as leading comments. Structured content keeps the matches (and the
/// commit) without the revision bytes the worker's envelope carries.
fn notation(tool: &str, response: Value) -> CallToolResult {
    let Ok(matches) =
        serde_json::from_value::<Vec<QueryMatchBlock>>(response["matches_after"].clone())
    else {
        return text(response, false);
    };
    let mut out = String::new();
    let mut structured = json!({ "matches": response["matches_after"] });
    if tool == "tonk_evaluate" {
        let claims = response["commits"]["claims"].as_u64().unwrap_or(0);
        out.push_str(&format!(
            "# committed {claims} claim{}\n",
            if claims == 1 { "" } else { "s" }
        ));
        if let Some(entities) = response["commits"]["entities"].as_object() {
            for (key, entity) in entities {
                out.push_str(&format!("# {key}: {}\n", entity.as_str().unwrap_or("?")));
            }
        }
        structured["commits"] = response["commits"].clone();
    }
    let body = render_matches(&matches);
    if body.is_empty() {
        out.push_str("# no matches\n");
    } else {
        let count: usize = matches.iter().map(|block| block.results.len()).sum();
        out.push_str(&body);
        out.push_str(&format!(
            "\n# {count} match{}\n",
            if count == 1 { "" } else { "es" }
        ));
    }
    let mut result = CallToolResult::success(vec![ContentBlock::text(out)]);
    result.structured_content = Some(structured);
    result
}

fn text(result: Value, error: bool) -> CallToolResult {
    let content = vec![ContentBlock::text(match &result {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    })];
    if error {
        CallToolResult::error(content)
    } else {
        let mut result_value = CallToolResult::success(content);
        // MCP requires structured content to be an object; strict clients
        // (Claude Desktop) reject the whole result otherwise.
        if result.is_object() {
            result_value.structured_content = Some(result);
        }
        result_value
    }
}

/// Tonk's app icon, inline: hosts show it next to the server's tools.
fn icon_data_uri() -> String {
    const PNG: &[u8] = include_bytes!("icon.png");
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::from("data:image/png;base64,");
    for chunk in PNG.chunks(3) {
        let bytes = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]);
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            out.push(if i <= chunk.len() {
                ALPHABET[(n >> shift & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

fn guide_topic(topic: &str) -> Option<&'static str> {
    if topic == "context" {
        Some(guide::AGENT)
    } else {
        guide::topic(topic)
    }
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(
            // Not plain "tonk": that is the hosted connector's name, and a
            // host may use only one server of a name.
            Implementation::new("tonk-peer", env!("CARGO_PKG_VERSION"))
                .with_title("Tonk")
                .with_website_url("https://github.com/tonk-labs/tonk/tree/main/plugins/tonk")
                .with_icons(vec![
                    Icon::new(icon_data_uri())
                        .with_mime_type("image/png")
                        .with_sizes(vec!["117x117".into()]),
                ]),
        )
        .with_instructions(guide::AGENT)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(self.tools.clone()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let name = request.name.as_ref();
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        if let Some(result) = self.views.call(&self.peer, name, &arguments).await {
            return Ok(result.into());
        }
        if !TOOLS.contains(&name) {
            return Ok(text(json!("Tool unavailable."), true).into());
        }
        if name == "tonk_guide" {
            let topic = arguments["topic"].as_str().unwrap_or("");
            return Ok(match (topic, guide_topic(topic)) {
                ("", _) => text(
                    json!({
                        "topics": guide::TOPICS,
                        "usage": "Call tonk_guide with a topic, or read tonk://guide/<topic>.",
                    }),
                    false,
                ),
                (_, Some(body)) => text(json!(body), false),
                (_, None) => text(json!("Unknown guide topic."), true),
            }
            .into());
        }
        Ok(match self.peer.call(name, arguments).await {
            Ok(result) if SCOPED.contains(&name) => notation(name, result),
            Ok(result) => text(result, false),
            Err(error) => text(json!(error), true),
        }
        .into())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let resources = std::iter::once("context")
            .chain(guide::TOPICS.iter().copied())
            .map(|topic| {
                Resource::new(
                    format!("tonk://guide/{topic}"),
                    format!("tonk-guide-{topic}"),
                )
                .with_mime_type("text/markdown")
            })
            .chain(self.views.resources())
            .collect::<Vec<_>>();
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        if let Some(contents) = self.views.read(&self.peer, &request.uri).await {
            let contents = contents.map_err(|error| McpError::internal_error(error, None))?;
            return Ok(ReadResourceResult::new(vec![contents]).into());
        }
        let body = request
            .uri
            .strip_prefix("tonk://guide/")
            .and_then(guide_topic)
            .ok_or_else(|| McpError::resource_not_found("Unknown Tonk guide.", None))?;
        Ok(ReadResourceResult::new(vec![
            ResourceContents::text(body, request.uri.clone()).with_mime_type("text/markdown"),
        ])
        .into())
    }
}

#[cfg(test)]
mod notation_tests {
    use super::*;

    fn response(claims: u64) -> Value {
        json!({
            "revision_before": { "tree": [1, 2, 3] },
            "revision_after": { "tree": [4, 5, 6] },
            "matches_before": [],
            "matches_after": [
                { "label": "counter", "results": [
                    { "this": "id:demo-counter", "fields": { "count": 32 } }
                ] }
            ],
            "commits": { "claims": claims, "entities": { "fresh": "id:new-thing" } },
        })
    }

    fn body(result: &CallToolResult) -> String {
        result.content[0].as_text().expect("text").text.clone()
    }

    #[test]
    fn a_query_answers_in_notation_without_the_envelope() {
        let result = notation("tonk_query", response(0));
        assert_eq!(
            body(&result),
            "counter:\n  this: id:demo-counter\n  count: 32\n# 1 match\n"
        );
        let structured = result.structured_content.expect("structured");
        assert_eq!(structured["matches"][0]["label"], "counter");
        assert!(structured.get("revision_after").is_none(), "{structured}");
    }

    #[test]
    fn a_write_leads_with_what_it_committed() {
        let result = notation("tonk_evaluate", response(2));
        assert_eq!(
            body(&result),
            "# committed 2 claims\n# fresh: id:new-thing\ncounter:\n  this: id:demo-counter\n  count: 32\n# 1 match\n"
        );
        assert_eq!(
            result.structured_content.expect("structured")["commits"]["claims"],
            2
        );
    }

    #[test]
    fn no_matches_says_so() {
        let mut empty = response(0);
        empty["matches_after"] = json!([]);
        assert_eq!(body(&notation("tonk_query", empty)), "# no matches\n");
    }

    #[test]
    fn an_unexpected_shape_passes_through() {
        let result = notation("tonk_query", json!({ "note": "not an evaluation" }));
        assert!(body(&result).contains("not an evaluation"));
    }
}
