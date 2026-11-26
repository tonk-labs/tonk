//! Pinned rmcp 3.5.1; fixture credentials arrive over stdin, never argv/logs.
use rmcp::ServiceExt;
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation, ProtocolVersion,
    ReadResourceRequestParams,
};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;

type Error = Box<dyn std::error::Error + Send + Sync>;

fn arguments(value: Value) -> serde_json::Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => unreachable!(),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let mut input = String::new();
    tokio::io::stdin().read_to_string(&mut input).await?;
    let config: Value = serde_json::from_str(&input)?;
    let (url, token, space) = (
        config["url"].as_str().ok_or("url")?,
        config["token"].as_str().ok_or("token")?,
        config["space"].as_str().ok_or("space")?,
    );
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(url).auth_header(token),
    );
    let info = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("tonk-mcp-rust-client", "0.0.0"),
    )
    .with_protocol_version(ProtocolVersion::V_2025_11_25);
    let session = info.serve(transport).await?;

    let initialized = session.peer_info().ok_or("no initialize result")?;
    assert_eq!(initialized.protocol_version, ProtocolVersion::V_2025_11_25);

    let mut tools: Vec<String> = session
        .list_all_tools()
        .await?
        .into_iter()
        .map(|tool| tool.name.into_owned())
        .collect();
    tools.sort();
    assert_eq!(
        tools,
        [
            "tonk_evaluate",
            "tonk_guide",
            "tonk_list_spaces",
            "tonk_query"
        ]
    );

    let spaces = session
        .call_tool(CallToolRequestParams::new("tonk_list_spaces"))
        .await?;
    let listed = spaces.structured_content.ok_or("no structuredContent")?;
    assert_eq!(listed["spaces"][0]["subject"], space);

    let resources = session.list_resources(None).await?;
    let first = resources.resources.first().ok_or("no resources")?;
    session
        .read_resource(ReadResourceRequestParams::new(first.uri.clone()))
        .await?;

    let written = session
        .call_tool(
            CallToolRequestParams::new("tonk_evaluate").with_arguments(arguments(json!({
                "space": space,
                "document": "integration-task!:\n  this: urn:test:rust\n  title: Rust SDK\n  done: false\n",
            }))),
        )
        .await?;
    assert_ne!(written.is_error, Some(true), "{:?}", written.content);

    let result = session
        .call_tool(
            CallToolRequestParams::new("tonk_query").with_arguments(arguments(json!({
                "space": space,
                "document": "integration-task:\n  this: urn:test:rust\n  title: ?title\n",
            }))),
        )
        .await?;
    assert_ne!(result.is_error, Some(true), "{:?}", result.content);
    let read = result.structured_content.ok_or("no structuredContent")?;
    assert!(serde_json::to_string(&read)?.contains("Rust SDK"));

    session.cancel().await?;
    println!("Rust rmcp 3.5.1 protocol 2025-11-25: list, guide, write, readback passed");
    Ok(())
}
