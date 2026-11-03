//! A network link between a client and a server on loopback, shaped like a
//! real one: round trip, jitter, bandwidth, stalls, resets, a network that
//! goes away, and a NAT that forgets a quiet connection.
//!
//! It works on TCP, below HTTP, so it carries whatever crosses it the same
//! way: page and service worker fetches, the socket live updates arrive
//! on, and presigned object reads and writes, whose signatures name the
//! host they were signed for and so cannot go through a proxy that
//! rewrites requests. Point the client at [`Link::url`] in place of the
//! server's own address.
//!
//! ```no_run
//! # async fn run() -> std::io::Result<()> {
//! use tonk_perf::link::{Link, Mode, Shape};
//!
//! let remote = Link::start("http://127.0.0.1:8787", Shape::profile("mobile").unwrap()).await?;
//! let url = format!("{}/ucan/", remote.url());
//! // ... sync against `url`, then stall it while a sync is in flight:
//! remote.set(Shape { mode: Mode::Stall, ..remote.shape() });
//! // ... and let it through again.
//! remote.set(Shape { mode: Mode::Open, ..remote.shape() });
//! println!("{:?}", remote.stats());
//! # Ok(())
//! # }
//! ```

mod control;
mod shape;
mod sniff;

pub use control::serve as serve_control;
pub use shape::{Mode, PROFILES, Shape};
pub use sniff::{Exchange, kind};

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::Write as _;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use shape::Draw;
use sniff::Exchanges;

/// How much a link reads at once.
const SEGMENT: usize = 16 * 1024;

/// How many exchanges a link keeps in memory for [`Link::exchanges`].
const KEPT: usize = 100_000;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|t| t.as_millis() as u64)
        .unwrap_or(0)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How to start a link.
#[derive(Debug, Clone)]
pub struct Options {
    /// The link's name, in its log and stats: `page`, `ucan`, `s3`, ...
    pub name: String,
    /// The port to listen on at 127.0.0.1; 0 picks one.
    pub listen: u16,
    /// Where it leads: `host:port` or a URL with them.
    pub upstream: String,
    pub shape: Shape,
}

/// What crossed a link so far.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Stats {
    pub name: String,
    pub shape: Shape,
    pub connections: u64,
    pub open: u64,
    pub refused: u64,
    pub resets: u64,
    pub stalls: u64,
    pub forgotten: u64,
    pub bytes_up: u64,
    pub bytes_down: u64,
    /// Requests seen, by [`kind`].
    pub exchanges: BTreeMap<String, u64>,
}

#[derive(Default)]
struct Counters {
    connections: AtomicU64,
    refused: AtomicU64,
    resets: AtomicU64,
    stalls: AtomicU64,
    forgotten: AtomicU64,
    bytes_up: AtomicU64,
    bytes_down: AtomicU64,
}

struct Inner {
    name: String,
    upstream: String,
    shape: watch::Sender<Shape>,
    /// The shape the link returns to when a gate opens it.
    base: Mutex<Shape>,
    next: AtomicU64,
    open: Mutex<HashMap<u64, Arc<Conn>>>,
    counters: Counters,
    /// When each direction's bandwidth is next free: up, then down.
    lanes: [Mutex<Instant>; 2],
    log: Option<Mutex<File>>,
    exchanges: Mutex<Vec<Exchange>>,
    kinds: Mutex<BTreeMap<String, u64>>,
}

/// A running link. Dropping it closes the link and resets every connection
/// across it.
pub struct Link {
    inner: Arc<Inner>,
    url: String,
    accept: tokio::task::AbortHandle,
}

/// A handle to change a link from elsewhere, such as the control server.
#[derive(Clone)]
pub struct Handle(Arc<Inner>);

impl Link {
    /// A link on a free port to `upstream`, shaped by `shape`.
    pub async fn start(upstream: &str, shape: Shape) -> std::io::Result<Self> {
        Self::start_with(
            Options {
                name: "link".into(),
                listen: 0,
                upstream: upstream.into(),
                shape,
            },
            None,
        )
        .await
    }

    /// A link as `options` say, logging each exchange to `log` as JSON
    /// lines when given.
    pub async fn start_with(options: Options, log: Option<&Path>) -> std::io::Result<Self> {
        let upstream = authority(&options.upstream).to_string();
        let listener = TcpListener::bind(("127.0.0.1", options.listen)).await?;
        let port = listener.local_addr()?.port();
        let log = match log {
            Some(path) => Some(Mutex::new(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)?,
            )),
            None => None,
        };
        let now = Instant::now();
        let inner = Arc::new(Inner {
            name: options.name,
            upstream,
            base: Mutex::new(options.shape.clone()),
            shape: watch::Sender::new(options.shape),
            next: AtomicU64::new(0),
            open: Mutex::default(),
            counters: Counters::default(),
            lanes: [Mutex::new(now), Mutex::new(now)],
            log,
            exchanges: Mutex::default(),
            kinds: Mutex::default(),
        });
        let accept = tokio::spawn(accept(listener, Arc::downgrade(&inner))).abort_handle();
        Ok(Self {
            inner,
            url: format!("http://127.0.0.1:{port}"),
            accept,
        })
    }

    /// The address to reach the upstream through this link.
    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn handle(&self) -> Handle {
        Handle(self.inner.clone())
    }

    pub fn shape(&self) -> Shape {
        self.handle().shape()
    }

    /// Shape the link as `shape` from now on. Bytes already on their way
    /// keep the delay they were given; a stall holds them and a switch to
    /// offline resets every connection.
    pub fn set(&self, shape: Shape) {
        self.handle().set(shape)
    }

    pub fn stats(&self) -> Stats {
        self.handle().stats()
    }

    /// Every exchange that ended so far, oldest first.
    pub fn exchanges(&self) -> Vec<Exchange> {
        lock(&self.inner.exchanges).clone()
    }

    /// Reset every connection now, as a network change does, leaving the
    /// link open to new ones.
    pub fn reset(&self) {
        self.handle().reset()
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.accept.abort();
        self.inner.reset();
    }
}

impl Handle {
    pub fn name(&self) -> &str {
        &self.0.name
    }

    pub fn shape(&self) -> Shape {
        self.0.shape.borrow().clone()
    }

    pub fn set(&self, shape: Shape) {
        let offline = shape.mode == Mode::Offline;
        self.0.shape.send_replace(shape);
        if offline {
            self.0.reset();
        }
    }

    pub fn reset(&self) {
        self.0.reset()
    }

    /// The shape the control's gate switches around.
    pub fn base(&self) -> Shape {
        lock(&self.0.base).clone()
    }

    pub fn set_base(&self, shape: Shape) {
        *lock(&self.0.base) = shape;
    }

    /// The exchanges still waiting on their answer, or still streaming
    /// it, as of now: what a stalled client is waiting on.
    pub fn inflight(&self) -> Vec<Exchange> {
        let open: Vec<Arc<Conn>> = lock(&self.0.open).values().cloned().collect();
        let mut pending: Vec<Exchange> = open
            .iter()
            .filter_map(|conn| lock(&conn.exchanges).pending(now_ms()))
            .collect();
        pending.sort_by_key(|exchange| exchange.t0);
        pending
    }

    pub fn stats(&self) -> Stats {
        let counters = &self.0.counters;
        let load = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        Stats {
            name: self.0.name.clone(),
            shape: self.shape(),
            connections: load(&counters.connections),
            open: lock(&self.0.open).len() as u64,
            refused: load(&counters.refused),
            resets: load(&counters.resets),
            stalls: load(&counters.stalls),
            forgotten: load(&counters.forgotten),
            bytes_up: load(&counters.bytes_up),
            bytes_down: load(&counters.bytes_down),
            exchanges: lock(&self.0.kinds).clone(),
        }
    }
}

impl Inner {
    fn reset(&self) {
        for (_, conn) in lock(&self.open).drain() {
            conn.token.cancel();
        }
    }

    fn record(&self, exchange: Exchange) {
        if let Some(log) = &self.log
            && let Ok(line) = serde_json::to_string(&exchange)
        {
            // One write per line, so links sharing a log never interleave.
            let _ = lock(log).write_all(format!("{line}\n").as_bytes());
        }
        *lock(&self.kinds).entry(exchange.kind.clone()).or_default() += 1;
        let mut kept = lock(&self.exchanges);
        if kept.len() < KEPT {
            kept.push(exchange);
        }
    }
}

/// `host:port` out of `upstream`, which may be a URL.
fn authority(upstream: &str) -> &str {
    let rest = upstream
        .split_once("://")
        .map_or(upstream, |(_, rest)| rest);
    rest.split('/').next().unwrap_or(rest)
}

/// Close `stream` with a reset rather than an orderly close.
fn abort(stream: &TcpStream) {
    let _ = stream.set_zero_linger();
}

async fn accept(listener: TcpListener, link: Weak<Inner>) {
    loop {
        let Ok((client, _)) = listener.accept().await else {
            continue;
        };
        let Some(link) = link.upgrade() else {
            return;
        };
        if link.shape.borrow().mode == Mode::Offline {
            link.counters.refused.fetch_add(1, Ordering::Relaxed);
            abort(&client);
            continue;
        }
        tokio::spawn(connection(link, client));
    }
}

/// What both directions of one connection share.
struct Conn {
    id: u64,
    accepted: Instant,
    /// When anything last crossed, either way.
    last: Mutex<Instant>,
    /// A NAT forgot the connection: nothing crosses it again.
    forgotten: AtomicBool,
    exchanges: Mutex<Exchanges>,
    draw: Mutex<Draw>,
    token: CancellationToken,
}

impl Conn {
    fn touch(&self) {
        *lock(&self.last) = Instant::now();
    }

    fn is_forgotten(&self) -> bool {
        self.forgotten.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Up,
    Down,
}

async fn connection(link: Arc<Inner>, client: TcpStream) {
    let id = link.next.fetch_add(1, Ordering::Relaxed) + 1;
    link.counters.connections.fetch_add(1, Ordering::Relaxed);
    let upstream = match TcpStream::connect(&link.upstream).await {
        Ok(upstream) => upstream,
        Err(_) => {
            abort(&client);
            return;
        }
    };
    let _ = client.set_nodelay(true);
    let _ = upstream.set_nodelay(true);
    let token = CancellationToken::new();
    let seed = link.shape.borrow().seed;
    let conn = Arc::new(Conn {
        id,
        accepted: Instant::now(),
        last: Mutex::new(Instant::now()),
        forgotten: AtomicBool::new(false),
        exchanges: Mutex::new(Exchanges::new(&link.name, id)),
        draw: Mutex::new(Draw::new(seed ^ id.wrapping_mul(0x9e37_79b9_7f4a_7c15))),
        token,
    });
    lock(&link.open).insert(id, conn.clone());

    let (client_read, client_write) = client.into_split();
    let (upstream_read, upstream_write) = upstream.into_split();
    let up = direction(
        link.clone(),
        conn.clone(),
        Direction::Up,
        client_read,
        upstream_write,
    );
    let down = direction(
        link.clone(),
        conn.clone(),
        Direction::Down,
        upstream_read,
        client_write,
    );
    let watchdog = tokio::spawn(forget_when_quiet(link.clone(), conn.clone()));
    tokio::join!(up, down);
    watchdog.abort();

    lock(&link.open).remove(&id);
    if let Some(ended) = lock(&conn.exchanges).closed(now_ms()) {
        link.record(ended);
    }
}

/// Mark `conn` forgotten once nothing crossed it for the shape's idle cut.
async fn forget_when_quiet(link: Arc<Inner>, conn: Arc<Conn>) {
    loop {
        let cut = link.shape.borrow().idle_cut;
        let tick = if cut.is_zero() {
            Duration::from_secs(1)
        } else {
            (cut / 4).clamp(Duration::from_millis(10), Duration::from_secs(1))
        };
        tokio::time::sleep(tick).await;
        if !cut.is_zero() && lock(&conn.last).elapsed() >= cut {
            conn.forgotten.store(true, Ordering::Relaxed);
            link.counters.forgotten.fetch_add(1, Ordering::Relaxed);
            return;
        }
    }
}

/// Carry one direction of `conn`: read segments, give each the time it
/// may be delivered, and deliver them in order at the link's bandwidth.
async fn direction(
    link: Arc<Inner>,
    conn: Arc<Conn>,
    direction: Direction,
    mut from: OwnedReadHalf,
    mut to: OwnedWriteHalf,
) {
    let (queue, mut queued) = mpsc::unbounded_channel::<(Instant, Option<Vec<u8>>)>();

    let reader = {
        let link = link.clone();
        let conn = conn.clone();
        async move {
            let mut released = conn.accepted;
            let mut first = true;
            let mut buffer = vec![0u8; SEGMENT];
            loop {
                let read = tokio::select! {
                    _ = conn.token.cancelled() => return,
                    read = from.read(&mut buffer) => read,
                };
                let segment = match read {
                    Ok(0) | Err(_) => None,
                    Ok(n) => Some(buffer[..n].to_vec()),
                };
                if conn.is_forgotten() {
                    // Nothing crosses a forgotten connection, not even its
                    // close: keep reading so the sender is not held up.
                    if segment.is_none() {
                        return;
                    }
                    continue;
                }
                let shape = link.shape.borrow().clone();
                let now = Instant::now();
                let mut at = now + shape.latency / 2;
                {
                    let mut draw = lock(&conn.draw);
                    at += draw.upto(shape.jitter);
                    if segment.is_some() && draw.chance(shape.stall_rate) {
                        link.counters.stalls.fetch_add(1, Ordering::Relaxed);
                        at += shape.stall_for;
                    }
                    if segment.is_some() && draw.chance(shape.reset_rate) {
                        link.counters.resets.fetch_add(1, Ordering::Relaxed);
                        lock(&link.open).remove(&conn.id);
                        conn.token.cancel();
                        return;
                    }
                }
                if first && direction == Direction::Up {
                    // The handshake costs a round trip before the first
                    // byte can be sent.
                    at = at.max(conn.accepted + shape.latency + shape.latency / 2);
                }
                first = false;
                // A segment never overtakes the one before it.
                at = at.max(released);
                released = at;
                if let Some(segment) = &segment {
                    conn.touch();
                    let counter = match direction {
                        Direction::Up => &link.counters.bytes_up,
                        Direction::Down => &link.counters.bytes_down,
                    };
                    counter.fetch_add(segment.len() as u64, Ordering::Relaxed);
                    if direction == Direction::Up
                        && let Some(ended) = lock(&conn.exchanges).sent(segment, now_ms())
                    {
                        link.record(ended);
                    }
                }
                let end = segment.is_none();
                if queue.send((at, segment)).is_err() || end {
                    return;
                }
            }
        }
    };

    let writer = async {
        let mut shape = link.shape.subscribe();
        let lane = match direction {
            Direction::Up => &link.lanes[0],
            Direction::Down => &link.lanes[1],
        };
        while let Some((at, segment)) = queued.recv().await {
            tokio::time::sleep_until(at).await;
            while shape.borrow_and_update().mode == Mode::Stall {
                if shape.changed().await.is_err() {
                    return;
                }
            }
            if conn.is_forgotten() {
                continue;
            }
            let Some(segment) = segment else {
                let _ = to.shutdown().await;
                return;
            };
            let transmit = shape.borrow().transmit(segment.len());
            if !transmit.is_zero() {
                let done = {
                    let mut free = lock(lane);
                    let start = (*free).max(Instant::now());
                    *free = start + transmit;
                    *free
                };
                tokio::time::sleep_until(done).await;
            }
            if to.write_all(&segment).await.is_err() {
                return;
            }
            conn.touch();
            if direction == Direction::Down {
                lock(&conn.exchanges).delivered(&segment, now_ms());
            }
        }
    };

    let token = conn.token.clone();
    tokio::select! {
        _ = token.cancelled() => {
            abort(to.as_ref());
        }
        _ = async { tokio::join!(reader, writer) } => {}
    }
}

/// The links a harness runs, by name, to look up from a control request.
pub type Links = Vec<Handle>;

/// Resolve `upstream` the way a link will, to fail early on a bad one.
pub fn check_upstream(upstream: &str) -> Result<SocketAddr, String> {
    use std::net::ToSocketAddrs as _;
    authority(upstream)
        .to_socket_addrs()
        .map_err(|error| format!("{upstream}: {error}"))?
        .next()
        .ok_or_else(|| format!("{upstream} resolves to nothing"))
}

#[cfg(test)]
mod test;
