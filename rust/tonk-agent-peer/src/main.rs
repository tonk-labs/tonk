//! A local Tonk peer for agent hosts (Claude Code, Cowork, Codex…), served as
//! a stdio MCP server.
//!
//! The peer has its own identity, generated on first use and kept in its data
//! directory. Share links are redeemed into it once with `tonk_join`; later
//! calls name a space. Every host process on a machine that uses the same data
//! directory is the same peer: the first to need it becomes the owner, keeps
//! the worker open and syncs in the background, and the rest forward their
//! calls to it over a private socket (see [`owner`]).
//!
//! Configuration, all optional:
//! - `TONK_AGENT_PEER_HOME`: data directory, default `~/.tonk/agent-peer`
use std::path::PathBuf;

use rmcp::ServiceExt;

#[path = "../../tonk-cli/src/guide.rs"]
#[allow(dead_code)]
mod guide;
mod owner;
mod server;

/// Keep the real stdout for the protocol and send fd 1 to stderr. The worker
/// and its dependencies print diagnostics to stdout, which on a stdio MCP
/// server would corrupt the stream the host reads.
fn protocol_stdout() -> std::io::Result<std::fs::File> {
    use std::os::fd::AsFd;
    let protocol = rustix::io::dup(std::io::stdout().as_fd())?;
    rustix::stdio::dup2_stdout(std::io::stderr().as_fd())?;
    Ok(std::fs::File::from(protocol))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protocol = tokio::fs::File::from_std(protocol_stdout()?);
    let root = match std::env::var_os("TONK_AGENT_PEER_HOME") {
        Some(home) => PathBuf::from(home),
        None => dirs::home_dir()
            .ok_or("No home directory; set TONK_AGENT_PEER_HOME.")?
            .join(".tonk")
            .join("agent-peer"),
    };
    let peer = owner::Peer::new(root);
    let service = server::Server::new(peer.clone())
        .serve((tokio::io::stdin(), protocol))
        .await?;
    // The host ends the session by closing stdin; a signal does the same.
    tokio::select! {
        _ = service.waiting() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
    peer.shutdown().await;
    Ok(())
}
