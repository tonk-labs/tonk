//! Ownership of one peer directory shared by several host processes.
//!
//! Hosts start an MCP server per session, so a machine may run several at
//! once against the same directory. Exactly one holds the directory lock: it
//! opens the worker, keeps it open, syncs in the background, and serves the
//! others over `peer.sock` inside the private directory. A process that does
//! not own the peer forwards each call; when the owner exits, the next call
//! anywhere takes over.
use std::{
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, UNIX_EPOCH},
};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::{Mutex, Notify},
    task::JoinHandle,
};
use tonk_worker::LocalPeer;

/// How often the owner pushes and pulls in the background, as the hosted
/// native worker does.
const SYNC_INTERVAL: Duration = Duration::from_secs(5);
/// Upper bound on one forwarded call, matching a slow pull plus evaluation.
const FORWARD_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Clone)]
pub struct Peer {
    root: PathBuf,
    role: Arc<Mutex<Role>>,
    // Whether this process already asked an owner to yield to its build.
    asked: Arc<AtomicBool>,
}

/// When this process's binary was installed. The launcher installs every
/// new build as a new file, so a later stamp is a newer build.
fn build_stamp() -> u128 {
    static STAMP: OnceLock<u128> = OnceLock::new();
    *STAMP.get_or_init(|| {
        std::env::current_exe()
            .and_then(std::fs::metadata)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|age| age.as_nanos())
            .unwrap_or(0)
    })
}

enum Role {
    Unclaimed,
    Owner {
        peer: Arc<Mutex<LocalPeer>>,
        // Where this process serves the peer's worker API to local views.
        origin: Arc<str>,
        tasks: Vec<JoinHandle<()>>,
        // Held for as long as this process owns the directory.
        _lock: std::fs::File,
    },
}

impl Peer {
    pub fn new(root: PathBuf) -> Self {
        // Read now: an update replaces the binary at the same path, and a
        // stamp read later would mistake this process for the newer build.
        build_stamp();
        Self {
            root,
            role: Arc::new(Mutex::new(Role::Unclaimed)),
            asked: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Run one tool on the peer, owning it if nobody else does.
    pub async fn call(&self, tool: &str, arguments: Value) -> Result<Value, String> {
        if let Some((peer, origin)) = self.owned().await? {
            return run(&peer, &origin, tool, &arguments).await;
        }
        // Once per process: an owner running an older build hands the
        // directory over, so an updated extension takes effect in the next
        // session without restarting the host.
        if !self.asked.swap(true, Ordering::SeqCst)
            && let Some((peer, origin)) = self.take_over().await?
        {
            return run(&peer, &origin, tool, &arguments).await;
        }
        match forward(&self.socket(), tool, &arguments).await {
            Ok(result) => result,
            // The owner went away between our lock check and the connection:
            // try once more, which claims ownership if it is free now.
            Err(Forward::Unreachable) => match self.owned().await? {
                Some((peer, origin)) => run(&peer, &origin, tool, &arguments).await,
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
        if let Role::Owner {
            peer, tasks, _lock, ..
        } = role
        {
            for task in tasks {
                task.abort();
            }
            let _ = std::fs::remove_file(self.socket());
            if let Ok(peer) = Arc::try_unwrap(peer) {
                peer.into_inner().close().await;
            }
        }
    }

    /// Ask the owner to yield if its build is older than ours, then claim
    /// the directory once it lets go.
    async fn take_over(&self) -> Result<Option<(Arc<Mutex<LocalPeer>>, Arc<str>)>, String> {
        let stamp = build_stamp().to_string();
        let answer = forward(
            &self.socket(),
            "tonk_peer_yield",
            &json!({ "stamp": stamp }),
        )
        .await;
        if !matches!(&answer, Ok(Ok(reply)) if reply["yielded"] == true) {
            return Ok(None);
        }
        for _ in 0..50 {
            if let Some(owned) = self.owned().await? {
                return Ok(Some(owned));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(None)
    }

    fn socket(&self) -> PathBuf {
        self.root.join("peer.sock")
    }

    /// The open peer when this process owns it, claiming it if it is free.
    async fn owned(&self) -> Result<Option<(Arc<Mutex<LocalPeer>>, Arc<str>)>, String> {
        let mut role = self.role.lock().await;
        if let Role::Owner { peer, origin, .. } = &*role {
            return Ok(Some((peer.clone(), origin.clone())));
        }
        let unavailable = |_| "Peer storage is unavailable.".to_owned();
        let Some(lock) = tonk_worker::try_lock_peer(&self.root).map_err(unavailable)? else {
            return Ok(None);
        };
        let peer = Arc::new(Mutex::new(LocalPeer::open(self.root.clone()).await?));
        let socket = self.socket();
        // A socket file left by a previous owner is stale: we hold the lock.
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).map_err(unavailable)?;
        let router = peer.lock().await.router();
        let (origin, loopback) = crate::loopback::start(router).await.map_err(unavailable)?;
        let origin: Arc<str> = origin.into();
        // A newer build asked us to step aside: release the directory and
        // retire this worker, then keep serving this session by forwarding.
        let yielded = Arc::new(Notify::new());
        {
            let me = self.clone();
            let yielded = yielded.clone();
            let peer = peer.clone();
            tokio::spawn(async move {
                yielded.notified().await;
                // Let the reply reach the new owner first.
                tokio::time::sleep(Duration::from_millis(100)).await;
                peer.lock().await.retire().await;
                me.shutdown().await;
            });
        }
        let tasks = vec![
            tokio::spawn(serve(listener, peer.clone(), origin.clone(), yielded)),
            tokio::spawn(background_sync(peer.clone())),
            loopback,
        ];
        *role = Role::Owner {
            peer: peer.clone(),
            origin: origin.clone(),
            tasks,
            _lock: lock,
        };
        Ok(Some((peer, origin)))
    }
}

/// Run a tool on the owned peer. Listing spaces also says which process owns
/// the peer, since every session on this machine talks to that one: an owner
/// left running from an older build answers with older code.
async fn run(
    peer: &Mutex<LocalPeer>,
    origin: &str,
    tool: &str,
    arguments: &Value,
) -> Result<Value, String> {
    // Views ask where the owner serves the worker API, whichever process
    // renders them.
    if tool == "tonk_peer_origin" {
        return Ok(json!({ "origin": origin }));
    }
    if tool == "tonk_view_log" {
        return Ok(json!({ "origin": origin, "log": crate::loopback::log() }));
    }
    let mut result = peer.lock().await.call(tool, arguments).await?;
    if tool == "tonk_list_spaces"
        && let Value::Object(fields) = &mut result
    {
        fields.insert(
            "owner".into(),
            json!({
                "version": env!("CARGO_PKG_VERSION"),
                "pid": std::process::id(),
                "binary": std::env::current_exe().ok(),
                "origin": origin,
            }),
        );
    }
    Ok(result)
}

/// Answer forwarded calls, one JSON line in and one JSON line out per
/// connection. The socket lives in the owner-only data directory.
async fn serve(
    listener: UnixListener,
    peer: Arc<Mutex<LocalPeer>>,
    origin: Arc<str>,
    yielded: Arc<Notify>,
) {
    while let Ok((stream, _)) = listener.accept().await {
        let peer = peer.clone();
        let origin = origin.clone();
        let yielded = yielded.clone();
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
                    if tool == "tonk_peer_yield" {
                        let theirs = arguments["stamp"]
                            .as_str()
                            .and_then(|stamp| stamp.parse::<u128>().ok())
                            .unwrap_or(0);
                        let yielding = theirs > build_stamp();
                        if yielding {
                            yielded.notify_one();
                        }
                        let reply = json!({ "result": { "yielded": yielding } });
                        let _ = write.write_all(format!("{reply}\n").as_bytes()).await;
                        return;
                    }
                    match run(&peer, &origin, &tool, &arguments).await {
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
    // Sync through the worker state, not the peer lock, so tool calls and
    // views never wait behind a pull. A failed pass (offline, remote down)
    // requeues its spaces for the next tick.
    let state = peer.lock().await.sync_state();
    let mut ticks = tokio::time::interval(SYNC_INTERVAL);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        tonk_worker::drain_sync(&state).await;
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
