//! What a kernel's origin answers beside the worker's API: how the kernel
//! says it is; the space's assets, which a page of the space stores with
//! `PUT /` and reads at `/asset:{hash}`, whole or in part; at any other
//! address, the content the space's routes keep there; and a page load of an
//! address a page of the space shows, with what the space keeps at `/`, the
//! page its renderer renders the address with. A persona answers a page load
//! of the profile with the profile's page. A persona also
//! answers its host, at `/_kernel/seed/{space}`, what a space it created is to
//! be created from, and takes `POST /_kernel/seed/{space}/settled` once the
//! space's own kernel did, and makes a space as a copy of one an earlier
//! release kept, which its host keeps, at `POST /_kernel/migrated`. A space's
//! kernel answers `/_kernel/snapshot` with
//! the space, for another to be made a copy of. A persona does for a space
//! what its own kernel asks through their hosts and only the persona can:
//! `POST /_kernel/space/{space}/invite`, `GET .../authority`,
//! `POST .../revoke` and `POST .../withdraw`.

use axum::body::Body;
use axum::http::{HeaderValue, Method, Request, Response, StatusCode, header};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use http_body_util::BodyExt as _;
use serde_json::{Value, json};

/// Which site the kernel serves: a space, by its DID, or the person's
/// profile.
#[derive(Clone)]
pub struct Site {
    pub space: Option<String>,
}

/// Where a request goes.
pub enum Routed {
    /// Answered here.
    Answer(Response<Body>),
    /// To the worker, whose answer is the stored blob's, to answer as an
    /// asset stored.
    Store(Request<Body>),
    /// To the worker, whose answer is the answer.
    Pass(Request<Body>),
    /// A page load of a whole tab, which the app shows at this address of
    /// its own: a site renders framed by the app.
    Away(String),
    /// A page load, to the worker: its answer if it has what is asked for,
    /// otherwise, where `viewed`, what the space keeps at `/`, or a page
    /// saying what it answered.
    Visit {
        request: Request<Body>,
        viewed: bool,
    },
    /// What the space, by its DID, is to be created from.
    #[cfg(feature = "persona")]
    Seed(String),
    /// The space, by its DID, was created from its seed.
    #[cfg(feature = "persona")]
    Settle(String),
    /// Make a space as a copy of one an earlier release kept, which the
    /// host keeps: the request names it and the space's name.
    #[cfg(feature = "persona")]
    Migrated(Request<Body>),
    /// The space, by its DID, for another to be made a copy of.
    Snapshot(String),
    /// What the kernel of the space, by its DID, asks the persona to do for
    /// it, and the request.
    #[cfg(feature = "persona")]
    SpaceAsk {
        space: String,
        ask: SpaceAsk,
        request: Request<Body>,
    },
}

/// What a space's kernel asks the persona to do for the space.
#[cfg(feature = "persona")]
pub enum SpaceAsk {
    /// Mint an invite to it.
    Invite,
    /// Answer the account's authority over it.
    Authority,
    /// Revoke a grant on it.
    Revoke,
    /// Take back the ticket it keeps for an audience.
    Withdraw,
}

/// The headers an asset's request keeps as it reaches the worker.
const STORED_HEADERS: [&str; 2] = ["content-type", "x-tonk-blob-name"];
const READ_HEADERS: [&str; 1] = ["range"];

/// Where `request` goes on `site`.
pub fn route(request: Request<Body>, site: &Site) -> Routed {
    let path = request.uri().path().to_owned();
    let method = request.method().clone();
    let reads = method == Method::GET || method == Method::HEAD;
    if path == "/api/health" && reads {
        return Routed::Answer(health(site));
    }
    let visits = method == Method::GET && !path.starts_with("/api/") && accepts_page(&request);
    if visits && whole_tab(&request) && asset(&path).is_none() {
        let query = request
            .uri()
            .query()
            .map(|query| format!("?{query}"))
            .unwrap_or_default();
        return Routed::Away(shown_at(site, &path, &query));
    }
    let Some(space) = &site.space else {
        // The profile keeps no content: every page load is its page.
        if visits {
            return Routed::Answer(profile_page());
        }
        #[cfg(feature = "persona")]
        if let Some(rest) = path.strip_prefix("/_kernel/space/")
            && let Some((space, asked)) = rest.split_once('/')
        {
            let ask = match (asked, &method) {
                ("invite", &Method::POST) => Some(SpaceAsk::Invite),
                ("authority", &Method::GET) => Some(SpaceAsk::Authority),
                ("revoke", &Method::POST) => Some(SpaceAsk::Revoke),
                ("withdraw", &Method::POST) => Some(SpaceAsk::Withdraw),
                _ => None,
            };
            if let Some(ask) = ask {
                return Routed::SpaceAsk {
                    space: space.to_owned(),
                    ask,
                    request,
                };
            }
        }
        #[cfg(feature = "persona")]
        if path == "/_kernel/migrated" && method == Method::POST {
            return Routed::Migrated(request);
        }
        #[cfg(feature = "persona")]
        if let Some(rest) = path.strip_prefix("/_kernel/seed/") {
            match (rest.strip_suffix("/settled"), method) {
                (Some(space), Method::POST) => return Routed::Settle(space.to_owned()),
                (None, Method::GET) => return Routed::Seed(rest.to_owned()),
                _ => {}
            }
        }
        return Routed::Pass(request);
    };
    if path == "/_kernel/snapshot" && method == Method::GET {
        return Routed::Snapshot(space.clone());
    }
    if path == "/" && method == Method::PUT {
        let target = format!("/api/repository/{space}/branch/main/blob");
        return Routed::Store(retarget(request, Method::POST, &target, &STORED_HEADERS));
    }
    if let Some(hash) = asset(&path)
        && reads
    {
        let target = format!("/api/repository/{space}/branch/main/blob/asset:{hash}");
        let request = retarget(request, method, &target, &READ_HEADERS);
        return reached(request, visits, false);
    }
    if reads && !path.starts_with("/api/") {
        let target = format!("/api/repository/{space}/branch/main/http{path}");
        let request = retarget(request, method, &target, &[]);
        return reached(request, visits, true);
    }
    Routed::Pass(request)
}

/// `request` to the worker, as a page load where `visits`.
fn reached(request: Request<Body>, visits: bool, viewed: bool) -> Routed {
    if visits {
        Routed::Visit { request, viewed }
    } else {
        Routed::Pass(request)
    }
}

/// Whether `request` loads a whole tab rather than a frame, as the browser
/// says in `sec-fetch-dest`.
fn whole_tab(request: &Request<Body>) -> bool {
    request
        .headers()
        .get("sec-fetch-dest")
        .is_some_and(|dest| dest == "document")
}

/// The address in the app that shows `path` of `site`: a space at its DID,
/// and the profile, being the account last signed in to, at the app's own.
fn shown_at(site: &Site, path: &str, query: &str) -> String {
    match &site.space {
        Some(space) => {
            let rest = if path == "/" { "" } else { path };
            format!("/{space}{rest}{query}")
        }
        None => format!("{path}{query}"),
    }
}

/// A redirect to `location`.
pub fn redirect(location: &str) -> Response<Body> {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::FOUND;
    if let Ok(location) = HeaderValue::from_str(location) {
        response.headers_mut().insert(header::LOCATION, location);
    }
    response
}

/// Whether `request` asks for a page, as a page load does.
fn accepts_page(request: &Request<Body>) -> bool {
    request
        .headers()
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|accept| accept.contains("text/html"))
}

/// The profile's page, which renders its view. A space keeps its own, with
/// the renderer it was created with.
#[cfg(feature = "persona")]
pub fn page() -> Response<Body> {
    html(StatusCode::OK, PAGE.to_owned())
}

#[cfg(feature = "persona")]
const PAGE: &str = include_str!("../assets/page.html");

/// What a page load of the profile is answered with: its page, which only a
/// persona serves.
fn profile_page() -> Response<Body> {
    #[cfg(feature = "persona")]
    return page();
    #[cfg(not(feature = "persona"))]
    {
        let mut nothing = Response::new(Body::empty());
        *nothing.status_mut() = StatusCode::NOT_FOUND;
        visited(nothing)
    }
}

/// The request for what `site` keeps at `/`, which a page load is answered
/// with where `viewed` and the worker's `reply` keeps nothing at the address
/// itself: the address is one a page of the space shows.
pub fn shown(reply: &Response<Body>, viewed: bool, site: &Site) -> Option<Request<Body>> {
    if !viewed || reply.status() != StatusCode::NOT_FOUND {
        return None;
    }
    let space = site.space.as_ref()?;
    Request::get(format!("/api/repository/{space}/branch/main/http/"))
        .body(Body::empty())
        .ok()
}

/// What a page load is answered with, from the worker's `reply`: the reply,
/// if it has what was asked for, or else a page that says what it answered.
pub fn visited(reply: Response<Body>) -> Response<Body> {
    let status = reply.status();
    if status.is_success() {
        return reply;
    }
    let said = match status {
        StatusCode::NOT_FOUND => "Nothing is here.".to_owned(),
        status if status.is_server_error() => "This space could not answer.".to_owned(),
        status => format!("This space answered {}.", status.as_u16()),
    };
    html(
        status,
        format!(
            r#"<!doctype html><html><head><meta charset="utf-8"><title>{code}</title><style>body{{font:14px/1.5 system-ui,sans-serif;margin:0;display:grid;place-items:center;min-height:100vh;opacity:.7}}</style></head><body><p>{said}</p></body></html>"#,
            code = status.as_u16(),
        ),
    )
}

/// A page that loads again in a moment, for what the kernel cannot answer
/// yet: where the app is, or the page of a space whose content has not
/// landed.
pub fn unready() -> Response<Body> {
    html(
        StatusCode::SERVICE_UNAVAILABLE,
        r#"<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="refresh" content="1"><title>503</title></head><body></body></html>"#
            .to_owned(),
    )
}

fn html(status: StatusCode, body: String) -> Response<Body> {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
}

/// The hash `path` names an asset by, `/asset:{hash}` in base58.
fn asset(path: &str) -> Option<&str> {
    let hash = path.strip_prefix("/asset:")?;
    let base58 = |byte: u8| byte.is_ascii_alphanumeric() && !b"0OIl".contains(&byte);
    (!hash.is_empty() && hash.bytes().all(base58)).then_some(hash)
}

/// `request`, as `method` to `target`, with only the headers in `kept`.
fn retarget(request: Request<Body>, method: Method, target: &str, kept: &[&str]) -> Request<Body> {
    let (parts, body) = request.into_parts();
    let mut built = Request::builder().method(method).uri(target);
    for name in kept {
        if let Some(value) = parts.headers.get(*name) {
            built = built.header(*name, value);
        }
    }
    built
        .body(body)
        .expect("a request of a method, a path and headers it was sent with")
}

/// How the kernel says it is: which site it serves, and, for a space, which
/// space it holds, so a page of its origin opened on its own finds its way
/// to the app.
fn health(site: &Site) -> Response<Body> {
    let said = match &site.space {
        Some(space) => json!({ "site": "space", "space": space, "worker": "ready" }),
        None => json!({ "site": "profile", "worker": "ready" }),
    };
    answer(StatusCode::OK, &said)
}

pub fn answer(status: StatusCode, body: &Value) -> Response<Body> {
    let mut response = Response::new(Body::from(body.to_string()));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

/// A space's snapshot: `content`, the CARv1, with `revision`, its JSON, in
/// base64 in `x-tonk-revision`.
pub fn snapshot(content: Vec<u8>, revision: &[u8]) -> Response<Body> {
    let mut response = Response::new(Body::from(content));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/vnd.ipld.car"),
    );
    if let Ok(revision) = HeaderValue::from_str(&BASE64.encode(revision)) {
        response.headers_mut().insert("x-tonk-revision", revision);
    }
    response
}

/// The worker's answer to storing a blob, as an asset stored: `201`, at the
/// address it is read at.
pub async fn stored(reply: Response<Body>) -> Response<Body> {
    if !reply.status().is_success() {
        return reply;
    }
    let (_, body) = reply.into_parts();
    let bytes = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(error) => {
            return answer(
                StatusCode::BAD_GATEWAY,
                &json!({ "error": format!("{error:?}") }),
            );
        }
    };
    let stored: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let Some(entity) = stored.get("entity").and_then(Value::as_str) else {
        return answer(
            StatusCode::BAD_GATEWAY,
            &json!({ "error": "the stored blob names no entity" }),
        );
    };
    let mut response = answer(StatusCode::CREATED, &stored);
    if let Ok(location) = HeaderValue::from_str(&format!("/{entity}")) {
        response.headers_mut().insert(header::LOCATION, location);
    }
    response
}
