//! The MCP surface. Tool definitions come from the shared catalog in
//! `mcp/tools.json`, so every Tonk host describes the same tools the same way;
//! guides are the CLI's canonical manuals.
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
        ListResourcesResult, ListToolsResult, PaginatedRequestParams, ReadResourceRequestParams,
        ReadResourceResponse, ReadResourceResult, Resource, ResourceContents, ServerCapabilities,
        ServerConfig, Tool,
    },
    service::RequestContext,
};
use serde_json::{Value, json};

use crate::{guide, owner::Peer};

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
}

impl Server {
    pub fn new(peer: Peer) -> Self {
        let catalog: Value = serde_json::from_str(CATALOG).expect("valid tool catalog");
        let tools = catalog["tools"]
            .as_array()
            .expect("catalog lists tools")
            .iter()
            .filter(|tool| TOOLS.contains(&tool["name"].as_str().unwrap_or_default()))
            .map(|tool| {
                let mut tool = tool.clone();
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
        Self { peer, tools }
    }
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
        result_value.structured_content = Some(result);
        result_value
    }
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
        .with_server_info(Implementation::new("tonk", env!("CARGO_PKG_VERSION")))
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
        if !TOOLS.contains(&name) {
            return Ok(text(json!("Tool unavailable."), true).into());
        }
        let arguments = Value::Object(request.arguments.unwrap_or_default());
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
                Resource::new(format!("tonk://guide/{topic}"), format!("tonk-guide-{topic}"))
                    .with_mime_type("text/markdown")
            })
            .collect::<Vec<_>>();
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
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
