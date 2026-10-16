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
//! - `TONK_PEER_HOME`: data directory, default `~/.tonk/peer`
//! - `TONK_PEER_REMOTE`: trusted access service, default the stable service
use std::path::PathBuf;

use rmcp::ServiceExt;

#[path = "../../tonk-cli/src/guide.rs"]
#[allow(dead_code)]
mod guide;
mod owner;
mod server;

const DEFAULT_REMOTE: &str = "https://tonk.network/ucan/";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = match std::env::var_os("TONK_PEER_HOME") {
        Some(home) => PathBuf::from(home),
        None => dirs::home_dir()
            .ok_or("No home directory; set TONK_PEER_HOME.")?
            .join(".tonk")
            .join("peer"),
    };
    let remote: url::Url = std::env::var("TONK_PEER_REMOTE")
        .unwrap_or_else(|_| DEFAULT_REMOTE.to_owned())
        .parse()?;
    let peer = owner::Peer::new(root, remote);
    let service = server::Server::new(peer.clone())
        .serve(rmcp::transport::stdio())
        .await?;
    // The host ends the session by closing stdin; a signal does the same.
    tokio::select! {
        _ = service.waiting() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
    peer.shutdown().await;
    Ok(())
}
