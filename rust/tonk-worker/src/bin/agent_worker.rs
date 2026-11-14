//! One JSON invocation on stdin, one marked JSON result on stdout. Credentials are
//! never command-line arguments. This executable has no listener.
use serde::Deserialize;
use std::io::{Read, Write};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    tool: String,
    agent_link: String,
    document: String,
}

#[tokio::main]
async fn main() {
    let result = run().await;
    let output = match result {
        Ok(value) => serde_json::json!({"result": value}),
        Err(error) => serde_json::json!({"error": error}),
    };
    // Native worker diagnostics also use stdout. The parent consumes only this
    // final marked line and never forwards raw worker output to the MCP client.
    let _ = writeln!(std::io::stdout(), "TONK_AGENT_LINK_RESULT {output}");
}

async fn run() -> Result<serde_json::Value, String> {
    let mut arguments = std::env::args().skip(1);
    let directory = arguments
        .next()
        .ok_or("Explicit storage directory required.")?;
    let remote = arguments
        .next()
        .ok_or("Trusted remote URL required.")?
        .parse()
        .map_err(|_| "Invalid trusted remote URL.")?;
    if arguments.next().is_some() {
        return Err("Unexpected arguments.".into());
    }
    let mut input = Vec::new();
    std::io::stdin()
        .take(200_001)
        .read_to_end(&mut input)
        .map_err(|_| "Could not read invocation.")?;
    if input.len() > 200_000 {
        return Err("Invocation too large.".into());
    }
    let input: Input = serde_json::from_slice(&input).map_err(|_| "Invalid invocation.")?;
    tonk_worker::AgentLinkWorker::new(directory.into(), remote)
        .call(&input.tool, &input.agent_link, &input.document)
        .await
}
