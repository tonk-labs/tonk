//! Caps on what one read answers, and paging past them.
//!
//! A space writer runs every read of its session in one Durable Object,
//! where CPU does not overlap and wasm memory never shrinks, so a read
//! answers at most [`ReadCaps::rows`] rows and [`ReadCaps::bytes`] bytes.
//! A read over either is refused with `READ_TOO_LARGE` unless it asks for
//! a page: `limit` rows from `cursor`, the opaque `next` of the page
//! before it. A page is cut from the read's whole answer, which is
//! computed again for each page, and a cursor names the revision it was
//! cut from, so a page of a space that changed since is refused with
//! `PAGE_STALE` rather than skipping or repeating rows.
//!
//! An answer is rows in one of the branch's two read shapes: an evaluate
//! (`matches_before` and `matches_after`, blocks of `results`; a read
//! does not commit, so the two are equal and a row counts once) or a
//! `/query` (an array of conclusions).

use axum::body::Body;
use axum::http::{Response, StatusCode, Uri, header};
use http_body_util::BodyExt as _;
use serde_json::{Map, Value};

use super::failure;

/// The default rows one read answers.
pub const MAX_READ_ROWS: usize = 1000;

/// The default bytes one read answers.
pub const MAX_READ_BYTES: usize = 1024 * 1024;

/// The most a read answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadCaps {
    /// Rows, counted across every block of an evaluate.
    pub rows: usize,
    /// Bytes of the answer's body.
    pub bytes: usize,
}

impl Default for ReadCaps {
    fn default() -> Self {
        Self {
            rows: MAX_READ_ROWS,
            bytes: MAX_READ_BYTES,
        }
    }
}

/// The page a read asks for, from its `limit` and `cursor` parameters.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Page {
    limit: Option<usize>,
    cursor: Option<String>,
}

impl Page {
    /// Take `limit` and `cursor` off `uri`, which goes on to the branch
    /// without them.
    pub(super) fn take(uri: &Uri) -> Result<(Self, Uri), String> {
        let mut page = Self::default();
        let mut rest = Vec::new();
        for pair in uri.query().unwrap_or("").split('&') {
            match pair.split_once('=').unwrap_or((pair, "")) {
                ("limit", value) => {
                    let limit = value
                        .parse::<usize>()
                        .ok()
                        .filter(|limit| *limit > 0)
                        .ok_or_else(|| {
                            format!("`limit` must be a positive number, not `{value}`")
                        })?;
                    page.limit = Some(limit);
                }
                ("cursor", value) if !value.is_empty() => page.cursor = Some(value.to_owned()),
                ("", _) => {}
                _ => rest.push(pair),
            }
        }
        let path = uri.path();
        let uri = if rest.is_empty() {
            path.to_owned()
        } else {
            format!("{path}?{}", rest.join("&"))
        };
        let uri = uri
            .parse()
            .map_err(|error: axum::http::uri::InvalidUri| error.to_string())?;
        Ok((page, uri))
    }

    fn requested(&self) -> bool {
        self.limit.is_some() || self.cursor.is_some()
    }
}

/// `response`, the answer to a read, held to `caps` and cut to `page`.
///
/// Anything but a successful JSON answer in a read shape passes as it is.
pub(super) async fn cap(response: Response<Body>, caps: ReadCaps, page: &Page) -> Response<Body> {
    if !response.status().is_success() || !is_json(&response) {
        return response;
    }
    let (parts, body) = response.into_parts();
    let body = match body.collect().await {
        Ok(body) => body.to_bytes(),
        Err(error) => {
            return failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "READ_UNREADABLE",
                &error.to_string(),
            );
        }
    };
    let Ok(mut answer) = serde_json::from_slice::<Value>(&body) else {
        return Response::from_parts(parts, Body::from(body));
    };
    let Some(rows) = rows(&answer) else {
        return Response::from_parts(parts, Body::from(body));
    };
    if !page.requested() {
        if rows > caps.rows || body.len() > caps.bytes {
            return too_large(rows, body.len(), caps);
        }
        return Response::from_parts(parts, Body::from(body));
    }

    let limit = page.limit.unwrap_or(caps.rows);
    if limit > caps.rows {
        return failure(
            StatusCode::BAD_REQUEST,
            "LIMIT_TOO_LARGE",
            &format!("`limit` is at most {} rows", caps.rows),
        );
    }
    let revision = fingerprint(&answer);
    let start = match &page.cursor {
        None => 0,
        Some(cursor) => match Cursor::parse(cursor) {
            Some(cursor) if cursor.revision == revision => cursor.offset,
            Some(_) => {
                return failure(
                    StatusCode::CONFLICT,
                    "PAGE_STALE",
                    "the space changed since this cursor was issued; read again from the first page",
                );
            }
            None => {
                return failure(
                    StatusCode::BAD_REQUEST,
                    "BAD_CURSOR",
                    "`cursor` is not one a page answered with",
                );
            }
        },
    };
    let end = start.saturating_add(limit).min(rows);
    slice(&mut answer, start.min(end), end);
    let next = (end < rows).then(|| {
        Cursor {
            offset: end,
            revision,
        }
        .to_string()
    });
    if let Value::Object(object) = &mut answer {
        object.insert(
            "page".into(),
            serde_json::json!({ "rows": rows, "next": next }),
        );
    } else {
        answer = serde_json::json!({ "rows": answer, "page": { "rows": rows, "next": next } });
    }
    let body = answer.to_string();
    if body.len() > caps.bytes {
        return failure(
            StatusCode::PAYLOAD_TOO_LARGE,
            "READ_TOO_LARGE",
            &format!(
                "this page of {} rows is {} bytes, over the cap of {} bytes; ask for a smaller `limit`",
                end - start,
                body.len(),
                caps.bytes
            ),
        );
    }
    let mut response = Response::from_parts(parts, Body::from(body));
    response.headers_mut().remove(header::CONTENT_LENGTH);
    response
}

fn is_json(response: &Response<Body>) -> bool {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/json"))
}

/// How many rows `answer` holds, if it is in a read shape.
fn rows(answer: &Value) -> Option<usize> {
    match answer {
        Value::Array(rows) => Some(rows.len()),
        Value::Object(object) => {
            let blocks = object.get("matches_before")?.as_array()?;
            Some(blocks.iter().map(|block| results(block).len()).sum())
        }
        _ => None,
    }
}

fn results(block: &Value) -> &[Value] {
    block["results"].as_array().map_or(&[], Vec::as_slice)
}

/// Keep rows `start..end` of `answer`, counted across its blocks in
/// order. A block keeps its label when none of its rows are kept.
fn slice(answer: &mut Value, start: usize, end: usize) {
    match answer {
        Value::Array(rows) => {
            rows.truncate(end);
            rows.drain(..start);
        }
        Value::Object(object) => {
            let mut seen = 0;
            let mut kept = Vec::new();
            for block in object["matches_before"].as_array().into_iter().flatten() {
                let results = results(block);
                let from = start.saturating_sub(seen).min(results.len());
                let to = end.saturating_sub(seen).min(results.len());
                seen += results.len();
                let mut block = block.as_object().cloned().unwrap_or_else(Map::new);
                block.insert("results".into(), Value::Array(results[from..to].to_vec()));
                kept.push(Value::Object(block));
            }
            // A read commits nothing, so its matches after are its
            // matches before.
            object.insert("matches_before".into(), Value::Array(kept.clone()));
            object.insert("matches_after".into(), Value::Array(kept));
        }
        _ => {}
    }
}

/// A short name for the revision `answer` was read at. A `/query`
/// answer names none, so its pages are not checked against a moved head.
fn fingerprint(answer: &Value) -> String {
    let revision = answer.get("revision_before").unwrap_or(&Value::Null);
    let hash = blake3::hash(revision.to_string().as_bytes());
    hash.to_hex()[..16].to_owned()
}

/// Where the next page starts: an offset into the answer at a revision.
struct Cursor {
    offset: usize,
    revision: String,
}

impl Cursor {
    fn parse(cursor: &str) -> Option<Self> {
        let (offset, revision) = cursor.split_once('.')?;
        Some(Self {
            offset: offset.parse().ok()?,
            revision: revision.to_owned(),
        })
    }
}

impl std::fmt::Display for Cursor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.offset, self.revision)
    }
}

fn too_large(rows: usize, bytes: usize, caps: ReadCaps) -> Response<Body> {
    failure(
        StatusCode::PAYLOAD_TOO_LARGE,
        "READ_TOO_LARGE",
        &format!(
            "this read answers {rows} rows in {bytes} bytes, over the cap of {} rows or {} \
             bytes; narrow the query, or read it in pages with `limit` and `cursor`",
            caps.rows, caps.bytes
        ),
    )
}
