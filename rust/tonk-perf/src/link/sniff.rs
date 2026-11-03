//! What a link can tell of the HTTP it carries in plaintext: one record per
//! request, timed as the client saw it.
//!
//! A browser does not pipeline: on one connection it sends a request,
//! reads the whole answer, then sends the next. So a segment from the
//! client that opens with a request line starts the next exchange, and
//! every byte until then belongs to the one before. An answer of `101`
//! turns the connection into a socket, whose frames are counted as one
//! exchange that lasts until it closes.

use serde::Serialize;

/// One request and its answer, as the client saw them.
#[derive(Debug, Clone, Serialize)]
pub struct Exchange {
    pub link: String,
    pub conn: u64,
    pub method: String,
    pub path: String,
    /// What the request is for, from the link and path: see [`kind`].
    pub kind: String,
    pub status: u16,
    /// When the request's first byte reached the link, in unix ms.
    pub t0: u64,
    /// When the answer's first byte reached the client.
    pub first_byte_ms: Option<u64>,
    /// When the answer's last byte reached the client, or the socket closed.
    pub t1: u64,
    pub req_bytes: u64,
    pub resp_bytes: u64,
}

/// What a request is for. The worker reaches the page origin (assets and
/// the worker's own routes) and the access service: UCAN redeems over
/// HTTP, the socket live updates arrive on, and the block and cell reads
/// and writes its permits allow, at `/object/`.
pub fn kind(link: &str, method: &str, path: &str, socket: bool) -> String {
    if socket {
        return "socket".into();
    }
    let route = path.split('?').next().unwrap_or(path);
    // The service answers a redeem with a permit for its own `/object/`
    // route (production's worker does the same), so block reads and
    // writes arrive there, or at S3 itself behind a link named for it.
    if link.starts_with("s3") || route.starts_with("/object/") {
        return match method {
            "GET" | "HEAD" => "object-get",
            "PUT" | "POST" => "object-put",
            "DELETE" => "object-delete",
            _ => "object-other",
        }
        .into();
    }
    // A UCAN site's address may be the service's root as well as its
    // `/ucan/` path: on the link to the service, what is not named
    // otherwise is a redeem.
    if route.starts_with("/ucan") || (link == "ucan" && method == "POST") {
        "ucan".into()
    } else if route.starts_with("/.well-known/") || route.starts_with("/customer/") {
        "service".into()
    } else if route.starts_with("/api/") {
        "api".into()
    } else {
        "asset".into()
    }
}

const METHODS: [&str; 7] = [
    "GET ", "POST ", "PUT ", "HEAD ", "DELETE ", "OPTIONS ", "PATCH ",
];

/// The method and path of a request line `segment` opens with, if any.
fn request_line(segment: &[u8]) -> Option<(String, String)> {
    if !METHODS.iter().any(|m| segment.starts_with(m.as_bytes())) {
        return None;
    }
    let end = segment.iter().position(|b| *b == b'\r' || *b == b'\n')?;
    let line = std::str::from_utf8(&segment[..end]).ok()?;
    let mut parts = line.split(' ');
    let (method, path, version) = (parts.next()?, parts.next()?, parts.next()?);
    version
        .starts_with("HTTP/1.")
        .then(|| (method.to_string(), path.to_string()))
}

/// The status of a response line `segment` opens with, if any.
fn status_line(segment: &[u8]) -> Option<u16> {
    let rest = segment.strip_prefix(b"HTTP/1.")?;
    let code = rest.get(2..5)?;
    std::str::from_utf8(code).ok()?.parse().ok()
}

/// The exchanges of one connection.
#[derive(Debug)]
pub(crate) struct Exchanges {
    link: String,
    conn: u64,
    current: Option<Exchange>,
    socket: bool,
}

impl Exchanges {
    pub(crate) fn new(link: &str, conn: u64) -> Self {
        Self {
            link: link.to_string(),
            conn,
            current: None,
            socket: false,
        }
    }

    /// The client sent `segment` at `now`; returns the exchange it ended,
    /// if it began another.
    pub(crate) fn sent(&mut self, segment: &[u8], now: u64) -> Option<Exchange> {
        if !self.socket
            && let Some((method, path)) = request_line(segment)
        {
            let ended = self.current.take();
            self.current = Some(Exchange {
                kind: kind(&self.link, &method, &path, false),
                link: self.link.clone(),
                conn: self.conn,
                method,
                path,
                status: 0,
                t0: now,
                first_byte_ms: None,
                t1: now,
                req_bytes: segment.len() as u64,
                resp_bytes: 0,
            });
            return ended;
        }
        if let Some(current) = &mut self.current {
            current.req_bytes += segment.len() as u64;
        }
        None
    }

    /// `segment` of the answer reached the client at `now`.
    pub(crate) fn delivered(&mut self, segment: &[u8], now: u64) {
        let Some(current) = &mut self.current else {
            return;
        };
        if current.first_byte_ms.is_none() {
            current.first_byte_ms = Some(now);
            current.status = status_line(segment).unwrap_or(0);
            if current.status == 101 {
                self.socket = true;
                current.kind = "socket".into();
            }
        }
        current.resp_bytes += segment.len() as u64;
        current.t1 = now;
    }

    /// The exchange still open, as of `now`, if any.
    pub(crate) fn pending(&self, now: u64) -> Option<Exchange> {
        let mut pending = self.current.clone()?;
        pending.t1 = now;
        Some(pending)
    }

    /// The connection closed at `now`: the exchange still open ends.
    pub(crate) fn closed(&mut self, now: u64) -> Option<Exchange> {
        let mut ended = self.current.take()?;
        if self.socket {
            ended.t1 = now;
        }
        Some(ended)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_splits_a_keep_alive_connection_into_exchanges() {
        let mut exchanges = Exchanges::new("ucan", 1);
        assert!(
            exchanges
                .sent(b"POST /ucan/ HTTP/1.1\r\nhost: x\r\n\r\n", 10)
                .is_none()
        );
        exchanges.sent(b"body", 11);
        exchanges.delivered(b"HTTP/1.1 200 OK\r\n\r\n", 50);
        exchanges.delivered(b"answer", 60);
        let first = exchanges
            .sent(b"GET /.well-known/tonk HTTP/1.1\r\n\r\n", 70)
            .expect("the second request ends the first");
        assert_eq!(first.kind, "ucan");
        assert_eq!(first.status, 200);
        assert_eq!(first.first_byte_ms, Some(50));
        assert_eq!(first.t1, 60);
        assert_eq!(
            first.req_bytes, 37,
            "the request line, its headers and its body"
        );
        assert_eq!(first.resp_bytes, 25);
        let second = exchanges.closed(80).expect("the open exchange ends");
        assert_eq!(second.kind, "service");
        assert_eq!(second.status, 0, "never answered");
    }

    #[test]
    fn a_socket_is_one_exchange_until_it_closes() {
        let mut exchanges = Exchanges::new("ucan", 2);
        exchanges.sent(b"GET /ucan/?sub=did:key:z HTTP/1.1\r\n\r\n", 0);
        exchanges.delivered(b"HTTP/1.1 101 Switching Protocols\r\n\r\n", 5);
        // A frame that happens to read like a request line is a frame.
        assert!(exchanges.sent(b"GET / HTTP/1.1\r\n", 6).is_none());
        exchanges.delivered(b"pong", 9);
        let socket = exchanges.closed(90_000).unwrap();
        assert_eq!(socket.kind, "socket");
        assert_eq!(socket.t1, 90_000);
    }

    #[test]
    fn object_storage_is_told_apart_by_method() {
        assert_eq!(kind("s3", "GET", "/bucket/x", false), "object-get");
        assert_eq!(kind("s3", "PUT", "/bucket/x", false), "object-put");
        assert_eq!(
            kind("ucan", "GET", "/object/a/b?permit=x", false),
            "object-get"
        );
        assert_eq!(kind("ucan", "POST", "/ucan/", false), "ucan");
        assert_eq!(kind("ucan", "POST", "/", false), "ucan");
        assert_eq!(kind("page", "GET", "/api/sync", false), "api");
        assert_eq!(kind("page", "GET", "/index.html", false), "asset");
    }
}
