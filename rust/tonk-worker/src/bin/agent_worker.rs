//! One JSON invocation on stdin, one marked JSON result on stdout. Credentials are
//! never command-line arguments. This mode has no listener.
//!
//! `serve` mode holds one sponsored agent's peer open while its views are
//! shown: it serves the agent's worker on loopback to the host (which passes
//! a token in `TONK_WORKER_HOST_TOKEN`) and takes the agent's tool calls as
//! JSON lines on stdin until stdin closes.
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
    #[serde(default)]
    call: Option<String>,
    #[serde(default)]
    space: Option<String>,
    #[serde(default)]
    document: Option<String>,
    /// A view route for `tonk_outline`.
    #[serde(default)]
    route: Option<String>,
}

/// One tool call to a served agent.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServedCall {
    id: u64,
    call: String,
    #[serde(default)]
    arguments: serde_json::Value,
}

#[tokio::main]
async fn main() {
    let mut arguments = std::env::args().skip(1);
    if let (Some(directory), Some(_remote), Some(mode), None) = (
        arguments.next(),
        arguments.next(),
        arguments.next(),
        arguments.next(),
    ) && mode == "serve"
    {
        if let Err(error) = serve(directory.into()).await {
            let _ = writeln!(
                std::io::stdout(),
                "TONK_AGENT_SERVE {}",
                serde_json::json!({ "error": error })
            );
            std::process::exit(1);
        }
        return;
    }
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
                "call": input.call,
                "space": input.space,
                "document": input.document,
                "route": input.route,
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

/// Serve one agent's copied directory until stdin closes.
async fn serve(directory: std::path::PathBuf) -> Result<(), String> {
    use tokio::io::AsyncBufReadExt as _;

    let token = std::env::var("TONK_WORKER_HOST_TOKEN")
        .ok()
        .filter(|token| token.len() >= 32)
        .ok_or("A private host token of at least 32 characters is required.")?;
    let agent = tonk_worker::ServedAgent::open(directory).await?;
    let router = tonk_worker::private_host(agent.router(), &token);
    let state = agent.sync_state();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| "Could not listen on loopback.")?;
    let address = listener
        .local_addr()
        .map_err(|_| "Could not listen on loopback.")?;
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    // Pull what others write, so open views update, and push what the
    // views write.
    let sync = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            tonk_worker::drain_sync(&state).await;
        }
    });
    let _ = writeln!(
        std::io::stdout(),
        "TONK_AGENT_SERVE {}",
        serde_json::json!({ "address": address.to_string() })
    );

    let agent = tokio::sync::Mutex::new(agent);
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let reply = match serde_json::from_str::<ServedCall>(&line) {
            Ok(call) => match agent.lock().await.call(&call.call, &call.arguments).await {
                Ok(result) => serde_json::json!({ "id": call.id, "result": result }),
                Err(error) => serde_json::json!({ "id": call.id, "error": error }),
            },
            Err(_) => serde_json::json!({ "error": "Invalid invocation." }),
        };
        let _ = writeln!(std::io::stdout(), "TONK_AGENT_CALL {reply}");
    }
    sync.abort();
    server.abort();
    agent.into_inner().close().await;
    Ok(())
}
