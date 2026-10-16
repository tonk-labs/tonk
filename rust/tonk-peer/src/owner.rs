//! Ownership of one peer directory shared by several host processes.
//!
//! Hosts start an MCP server per session, so a machine may run several at
//! once against the same directory. Exactly one holds the directory lock: it
//! opens the worker, keeps it open, syncs in the background, and serves the
//! others over `peer.sock` inside the private directory. A process that does
//! not own the peer forwards each call; when the owner exits, the next call
//! anywhere takes over.
use std::{path::PathBuf, sync::Arc, time::Duration};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::Mutex,
    task::JoinHandle,
};
use tonk_worker::LocalPeer;

/// How often the owner pushes and pulls in the background.
const SYNC_INTERVAL: Duration = Duration::from_secs(30);
/// Upper bound on one forwarded call, matching a slow pull plus evaluation.
const FORWARD_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Clone)]
pub struct Peer {
    root: PathBuf,
    remote: url::Url,
    role: Arc<Mutex<Role>>,
}

enum Role {
    Unclaimed,
    Owner {
        peer: Arc<Mutex<LocalPeer>>,
        tasks: Vec<JoinHandle<()>>,
        // Held for as long as this process owns the directory.
        _lock: std::fs::File,
    },
}

impl Peer {
    pub fn new(root: PathBuf, remote: url::Url) -> Self {
        Self {
            root,
            remote,
            role: Arc::new(Mutex::new(Role::Unclaimed)),
        }
    }

    /// Run one tool on the peer, owning it if nobody else does.
    pub async fn call(&self, tool: &str, arguments: Value) -> Result<Value, String> {
        if let Some(peer) = self.owned().await? {
            return peer.lock().await.call(tool, &arguments).await;
        }
        match forward(&self.socket(), tool, &arguments).await {
            Ok(result) => result,
            // The owner went away between our lock check and the connection:
            // try once more, which claims ownership if it is free now.
            Err(Forward::Unreachable) => match self.owned().await? {
                Some(peer) => peer.lock().await.call(tool, &arguments).await,
                None => Err("The local peer is busy in another session; try again.".into()),
            },
            Err(Forward::Lost) => Err(
                "The local peer did not answer. If this was a write, query before repeating it."
                    .into(),
            ),
        }
    }

    /// Release ownership so another session can take over.
    pub async fn shutdown(&self) {
        let role = std::mem::replace(&mut *self.role.lock().await, Role::Unclaimed);
        if let Role::Owner { peer, tasks, _lock } = role {
            for task in tasks {
                task.abort();
            }
            let _ = std::fs::remove_file(self.socket());
            if let Ok(peer) = Arc::try_unwrap(peer) {
                peer.into_inner().close().await;
            }
        }
    }

    fn socket(&self) -> PathBuf {
        self.root.join("peer.sock")
    }

    /// The open peer when this process owns it, claiming it if it is free.
    async fn owned(&self) -> Result<Option<Arc<Mutex<LocalPeer>>>, String> {
        let mut role = self.role.lock().await;
        if let Role::Owner { peer, .. } = &*role {
            return Ok(Some(peer.clone()));
        }
        let unavailable = |_| "Peer storage is unavailable.".to_owned();
        let Some(lock) = tonk_worker::try_lock_peer(&self.root).map_err(unavailable)? else {
            return Ok(None);
        };
        let peer = Arc::new(Mutex::new(
            LocalPeer::open(self.root.clone(), self.remote.clone()).await?,
        ));
        let socket = self.socket();
        // A socket file left by a previous owner is stale: we hold the lock.
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).map_err(unavailable)?;
        let tasks = vec![
            tokio::spawn(serve(listener, peer.clone())),
            tokio::spawn(background_sync(peer.clone())),
        ];
        *role = Role::Owner {
            peer: peer.clone(),
            tasks,
            _lock: lock,
        };
        Ok(Some(peer))
    }
}

/// Answer forwarded calls, one JSON line in and one JSON line out per
/// connection. The socket lives in the owner-only data directory.
async fn serve(listener: UnixListener, peer: Arc<Mutex<LocalPeer>>) {
    while let Ok((stream, _)) = listener.accept().await {
        let peer = peer.clone();
        tokio::spawn(async move {
            let (read, mut write) = stream.into_split();
            let mut line = String::new();
            if BufReader::new(read).read_line(&mut line).await.is_err() {
                return;
            }
            let reply = match serde_json::from_str::<Value>(&line) {
                Ok(request) => {
                    let tool = request["tool"].as_str().unwrap_or_default().to_owned();
                    let arguments = request["arguments"].clone();
                    match peer.lock().await.call(&tool, &arguments).await {
                        Ok(result) => json!({ "result": result }),
                        Err(error) => json!({ "error": error }),
                    }
                }
                Err(_) => json!({ "error": "Invalid forwarded call." }),
            };
            let _ = write.write_all(format!("{reply}\n").as_bytes()).await;
        });
    }
}

/// Keep joined spaces current while any session has the plugin open.
async fn background_sync(peer: Arc<Mutex<LocalPeer>>) {
    let mut ticks = tokio::time::interval(SYNC_INTERVAL);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        // A failed pass (offline, remote down) is retried on the next tick.
        let _ = peer.lock().await.sync().await;
    }
}

enum Forward {
    /// No owner is listening; nothing was sent.
    Unreachable,
    /// The call was sent but no answer arrived; its outcome is unknown.
    Lost,
}

async fn forward(
    socket: &std::path::Path,
    tool: &str,
    arguments: &Value,
) -> Result<Result<Value, String>, Forward> {
    let stream = UnixStream::connect(socket)
        .await
        .map_err(|_| Forward::Unreachable)?;
    let exchange = async {
        let (read, mut write) = stream.into_split();
        let request = json!({ "tool": tool, "arguments": arguments });
        write
            .write_all(format!("{request}\n").as_bytes())
            .await
            .map_err(|_| Forward::Unreachable)?;
        let mut line = String::new();
        BufReader::new(read)
            .read_line(&mut line)
            .await
            .map_err(|_| Forward::Lost)?;
        let reply: Value = serde_json::from_str(&line).map_err(|_| Forward::Lost)?;
        Ok(match reply.get("error").and_then(Value::as_str) {
            Some(error) => Err(error.to_owned()),
            None => Ok(reply["result"].clone()),
        })
    };
    tokio::time::timeout(FORWARD_TIMEOUT, exchange)
        .await
        .map_err(|_| Forward::Lost)?
}
