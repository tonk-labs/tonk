//! One JSON invocation on stdin, one marked JSON result on stdout. Credentials are
//! never command-line arguments. This executable has no listener.
use serde::Deserialize;
use std::io::{Read, Write};

/// A call authorized by an agent link, or a call on a hosted agent.
#[derive(Deserialize)]
#[serde(untagged)]
enum Input {
    Link(LinkInput),
    Agent(AgentInput),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkInput {
    tool: String,
    agent_link: String,
    document: String,
}

/// `agent_create`, `agent_identity {agent}` or
/// `agent_sponsor {agent, sponsorship_hex, recipient}`. Agents live under
/// `<directory>/agents`, apart from link replicas.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentInput {
    tool: String,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    sponsorship_hex: Option<String>,
    #[serde(default)]
    recipient: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    agent_name: Option<String>,
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
    match serde_json::from_slice(&input).map_err(|_| "Invalid invocation.")? {
        Input::Link(input) => {
            tonk_worker::AgentLinkWorker::new(directory.into(), remote)
                .call(&input.tool, &input.agent_link, &input.document)
                .await
        }
        Input::Agent(input) => {
            let arguments = serde_json::json!({
                "agent": input.agent,
                "sponsorship_hex": input.sponsorship_hex,
                "recipient": input.recipient,
                "name": input.name,
                "agent_name": input.agent_name,
            });
            tonk_worker::HostedAgents::new(
                std::path::PathBuf::from(directory).join("agents"),
                remote,
            )
            .call(&input.tool, &arguments)
            .await
        }
    }
}
