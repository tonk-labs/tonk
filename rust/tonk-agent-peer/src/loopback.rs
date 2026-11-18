//! The owner's worker API on a loopback port, for Tonk views rendered in the
//! host's conversation (MCP Apps). Every request must carry this process's
//! random token in `x-tonk-peer-token`; the token reaches only the views the
//! host loads from this server's app resources, so other local processes and
//! web pages that find the port cannot act as this peer.
//!
//! The view page relays the sealed guest's requests here, the way the browser
//! relays them to its service worker. Besides the worker routes, it serves the
//! guest runtime pack (`space-runtime.json`) and the guest document
//! (`worker-guest.html`) from the runtime directory shipped beside the binary.
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    Router,
    body::Body,
    extract::Request,
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use tokio::{net::TcpListener, task::JoinHandle};

/// Bind a loopback port and serve `router` on it. Returns the origin and the
/// serving task.
pub async fn start(root: &Path, router: Router) -> std::io::Result<(String, JoinHandle<()>)> {
    load_token(root)?;
    let listener = bind(root).await?;
    let origin = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let app = Router::new()
        .route("/space-runtime.json", get(|| asset("space-runtime.json")))
        .route("/worker-guest.html", get(|| asset("worker-guest.html")))
        .route("/display-guest.html", get(display_guest))
        .route("/agent-peer/probe", get(|| async { "ok" }))
        .route(
            "/agent-peer/log",
            axum::routing::post(|body: String| async move {
                record(format!("view: {body}"));
                StatusCode::NO_CONTENT
            }),
        )
        .merge(router)
        .layer(middleware::from_fn(client_id))
        .layer(middleware::from_fn(trace))
        .layer(middleware::from_fn(authorize))
        .layer(middleware::from_fn(cors));
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((origin, task))
}

/// Recent loopback requests and view steps, newest last. Spike diagnostics:
/// a blank view is explained by reading this through `tonk_view_log`.
fn journal() -> &'static Mutex<VecDeque<String>> {
    static JOURNAL: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
    JOURNAL.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn record(line: String) {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_secs())
        .unwrap_or(0);
    let mut journal = journal().lock().expect("journal");
    if journal.len() >= 300 {
        journal.pop_front();
    }
    journal.push_back(format!("{seconds} {line}"));
}

/// The recorded lines, oldest first.
pub fn log() -> Vec<String> {
    journal().lock().expect("journal").iter().cloned().collect()
}

async fn trace(request: Request, next: Next) -> Response {
    let line = format!("{} {}", request.method(), request.uri());
    let started = Instant::now();
    let response = next.run(request).await;
    if !line.ends_with("/agent-peer/log") {
        record(format!(
            "{line} -> {} in {}ms",
            response.status().as_u16(),
            started.elapsed().as_millis()
        ));
    }
    response
}

/// Where the guest runtime lives: `TONK_AGENT_PEER_RUNTIME`, else `runtime/`
/// beside the binary (how the extension ships it).
pub fn runtime_dir() -> Option<PathBuf> {
    std::env::var_os("TONK_AGENT_PEER_RUNTIME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_exe()
                .ok()?
                .parent()
                .map(|dir| dir.join("runtime"))
        })
}

async fn asset(name: &'static str) -> Response {
    let content_type = if name.ends_with(".html") {
        "text/html; charset=utf-8"
    } else {
        "application/json"
    };
    match runtime_dir().map(|dir| dir.join(name)) {
        Some(path) => match tokio::fs::read(&path).await {
            Ok(bytes) => Response::builder()
                .header(header::CONTENT_TYPE, content_type)
                .header(header::CACHE_CONTROL, "no-cache")
                .body(Body::from(bytes))
                .expect("asset response"),
            Err(_) => missing(name),
        },
        None => missing(name),
    }
}

/// Mounts one `<tonk-display>` per entity, each with its concept's own
/// display, routed to the space. Runs before the bootstrap scripts, like the
/// hosted embed's mount script.
const DISPLAY_MOUNT: &str = r#"
// Spike diagnostics: the element runtime reports unresolved command
// bindings only with console.warn, which never leaves the guest.
for (const level of ['warn', 'error']) {
  const original = console[level].bind(console);
  console[level] = (...args) => {
    original(...args);
    try {
      parent.postMessage({ __tonkRuntime: 'warn', level, error: args.map(String).join(' ').slice(0, 1000) }, '*');
    } catch {}
  };
}
addEventListener('message', (e) => {
  if (e.source !== parent || e.data?.type !== 'tonk-display-mount') return;
  window.__tonkHideFab = true;
  const style = document.createElement('style');
  style.textContent = 'tonk-fab{display:none!important} body{padding:8px;display:flex;flex-direction:column;gap:12px}';
  document.head.append(style);
  document.body.replaceChildren(...e.data.items.map((item) => {
    const display = document.createElement('tonk-display');
    display.setAttribute('with', 'main@' + e.data.subject);
    display.setAttribute('entity', item.entity);
    display.setAttribute('model', item.model);
    if (item.view) display.setAttribute('view', item.view);
    // Report when the command delegate is installed, and which bindings it has.
    display.addEventListener('tonk-display:bound', () => {
      const bindings = [...display.querySelectorAll('[data-event-bindings]')]
        .map((view) => view.getAttribute('data-event-bindings')).join(' ');
      parent.postMessage({ __tonkRuntime: 'warn', level: 'info', error: 'bound ' + item.entity + ' ' + bindings.slice(0, 600) }, '*');
    });
    return display;
  }));
  // The host sizes the view to what it reports, and only this document can
  // measure its content: report its height whenever it changes.
  let reported = 0;
  new ResizeObserver(() => {
    const height = Math.ceil(document.documentElement.getBoundingClientRect().height);
    if (height === reported) return;
    reported = height;
    parent.postMessage({ type: 'tonk-display-size', height }, '*');
  }).observe(document.documentElement);
});
"#;

/// The guest document for displays: the hosted guest's policy and bootstrap
/// scripts from the runtime pack, with [`DISPLAY_MOUNT`] in place of the
/// whole-space mount.
async fn display_guest() -> Response {
    let Some(path) = runtime_dir().map(|dir| dir.join("space-runtime.json")) else {
        return missing("space-runtime.json");
    };
    let Ok(bytes) = tokio::fs::read(&path).await else {
        return missing("space-runtime.json");
    };
    let Ok(pack) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return missing("space-runtime.json");
    };
    let script = |source: &str| {
        format!(
            "<script>{}</script>",
            source.replace("</script", "<\\/script")
        )
    };
    let field = |name: &str| pack[name].as_str().unwrap_or_default().to_owned();
    let html = format!(
        "<!doctype html><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval' blob:; style-src 'unsafe-inline' blob:; img-src data: blob:; font-src data: blob:; connect-src blob:; frame-src blob: about:; form-action 'none'\"><style>html,body{{margin:0;font-family:system-ui}}</style>{}{}{}{}<body></body>",
        script(&field("theme")),
        script(DISPLAY_MOUNT),
        script(&field("bootstrap")),
        script(&field("runtimeBootstrap")),
    );
    Response::builder()
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from(html))
        .expect("guest response")
}

fn missing(name: &str) -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from(format!(
            "The Tonk guest runtime ({name}) is not installed beside this peer."
        )))
        .expect("missing response")
}

/// Hosts start and stop server processes freely, so ownership of a peer
/// moves between them while a view is open, and a host may keep a view's page
/// from an earlier read. Both the address and the token a view was given must
/// therefore outlive the process that gave them: the port is derived from the
/// data directory, and the token is kept in it.
async fn bind(root: &Path) -> std::io::Result<TcpListener> {
    // FNV-1a over the directory path: stable across runs and builds.
    let seed = root
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
        });
    let preferred = 49152 + (seed % 16000) as u16;
    for port in preferred..preferred + 8 {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)).await {
            return Ok(listener);
        }
    }
    // Something else holds those ports: views opened before this owner
    // will need reopening.
    TcpListener::bind(("127.0.0.1", 0)).await
}

static TOKEN: OnceLock<String> = OnceLock::new();

/// Read the views' token from the private data directory, creating it on
/// first use. Anything that can read that directory can already act as the
/// peer, so keeping the token beside the peer's key adds no exposure.
fn load_token(root: &Path) -> std::io::Result<()> {
    use std::io::{Read, Write};
    let path = root.join("view-token");
    let token = match std::fs::read_to_string(&path) {
        Ok(token) if token.trim().len() == 64 => token.trim().to_owned(),
        _ => {
            let mut bytes = [0u8; 32];
            std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
            let token: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
            let mut file = std::fs::OpenOptions::new();
            file.write(true).create(true).truncate(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut file, 0o600);
            file.open(&path)?.write_all(token.as_bytes())?;
            token
        }
    };
    let _ = TOKEN.set(token);
    Ok(())
}

/// The secret that views present on every request.
pub fn token() -> &'static str {
    TOKEN.get().map(String::as_str).unwrap_or_default()
}

/// Refuse any request without the token. Preflights never reach here: the
/// browser sends them without custom headers, and [`cors`] answers them.
async fn authorize(request: Request, next: Next) -> Response {
    let presented = request
        .headers()
        .get("x-tonk-peer-token")
        .and_then(|value| value.to_str().ok());
    if !token().is_empty()
        && presented
            .is_some_and(|presented| constant_time_eq(presented.as_bytes(), token().as_bytes()))
    {
        return next.run(request).await;
    }
    record(format!(
        "{} {} -> 401 ({} token)",
        request.method(),
        request.uri().path(),
        if presented.is_some() { "wrong" } else { "no" }
    ));
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .body(Body::from("This peer only answers its own views."))
        .expect("refusal response")
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

/// The worker tags subscriptions and site entities by client; the view page
/// names itself with `x-tonk-client-id`, as the hosted worker expects.
async fn client_id(mut request: Request, next: Next) -> Response {
    if let Some(client) = request
        .headers()
        .get("x-tonk-client-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
    {
        request
            .extensions_mut()
            .insert(tonk_worker::ClientId(client));
    }
    next.run(request).await
}

/// The view page runs at an opaque origin inside the host, so every response
/// allows any origin, and preflights include Private Network Access.
async fn cors(request: Request, next: Next) -> Response {
    let asked = request
        .headers()
        .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
        .cloned();
    let mut response = if request.method() == Method::OPTIONS {
        Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(Body::empty())
            .expect("preflight response")
    } else {
        next.run(request).await
    };
    allow(response.headers_mut(), asked);
    response
}

fn allow(headers: &mut HeaderMap, asked: Option<HeaderValue>) {
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, PUT, DELETE, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        asked.unwrap_or(HeaderValue::from_static("*")),
    );
    headers.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        "access-control-allow-private-network",
        HeaderValue::from_static("true"),
    );
}
