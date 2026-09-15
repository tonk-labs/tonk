//! Permitted object operations, performed over the R2 binding.
//!
//! `/object/{key}` is where a `/ucan/` answer is redeemed. The request
//! carries a service-signed permit in its query string (see
//! [`crate::permit`]); once that verifies, the operation it names is
//! performed against the bucket this worker is bound to, and the bytes
//! are streamed back. Nothing about the operation is read from the
//! request beyond the permit and, for a read, the `Range` header: the
//! key, the method, the checksum a write must match and the precondition
//! it must satisfy all come out of the verified claims.
//!
//! The answers keep the shape the client already understands from the
//! S3 endpoint this replaces: `ETag` on reads and writes, `404` for an
//! absent object, `412` for a precondition that did not hold, `206`
//! with `Content-Range` for a ranged read, and `401`/`403` for a permit
//! that is not good here, which is what tells the client to redeem a
//! fresh one.
//!
//! Neither direction is buffered. A read is handed to the runtime as
//! the binding's own stream. A write flows chunk by chunk into the
//! binding through a fixed-length stream, hashed on the way past; R2
//! verifies the bound checksum itself and refuses to store a body that
//! does not match, and the hash taken here is what tells that refusal
//! apart from a store that failed for its own reasons. A blob is as
//! large as it is, and the worker's memory is not what bounds it.

use std::cell::RefCell;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use crate::observability::{
    AccessFailureKind, AccessFailureLog, AccessOperation, AccessOutcome, AccessSite,
};
use crate::permit::{Claims, Method, PermitRefusal, Precondition};
use futures_util::Stream;
use sha2_0_10::{Digest, Sha256};
use worker::js_sys::Uint8Array;
use worker::wasm_bindgen::JsValue;
use worker::{
    Bucket, Data, Date, FixedLengthStream, Headers, Range, Request, Response, Result, RouteContext,
    console_error,
};

/// Add CORS headers for the object path.
///
/// Reads need no header at all, so a cross-origin read is a simple
/// request and never preflights. A ranged read adds `Range` and a
/// write is a `PUT`, both of which preflight once per URL, the same as
/// the presigned S3 URLs did.
fn with_cors_headers(response: Response) -> Response {
    let headers = response.headers().clone();
    let _ = headers.set("Access-Control-Allow-Origin", "*");
    let _ = headers.set("Access-Control-Allow-Methods", "GET, PUT, DELETE, OPTIONS");
    let _ = headers.set("Access-Control-Allow-Headers", "Content-Type, Range");
    let _ = headers.set(
        "Access-Control-Expose-Headers",
        "ETag, Content-Length, Content-Range",
    );
    response.with_headers(headers)
}

/// OPTIONS /object/{key} → CORS preflight.
pub async fn handle_options(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    let response = with_cors_headers(Response::empty()?.with_status(204));
    let headers = response.headers().clone();
    let _ = headers.set("Access-Control-Max-Age", crate::PREFLIGHT_MAX_AGE);
    Ok(response.with_headers(headers))
}

/// GET /object/{key} → the object, or a byte range of it.
pub async fn handle_get(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    serve(req, ctx, Method::Get).await
}

/// PUT /object/{key} → store the body as the object.
pub async fn handle_put(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    serve(req, ctx, Method::Put).await
}

/// DELETE /object/{key} → remove the object.
pub async fn handle_delete(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    serve(req, ctx, Method::Delete).await
}

async fn serve(mut req: Request, ctx: RouteContext<()>, method: Method) -> Result<Response> {
    let response = match perform(&mut req, &ctx, method).await {
        Ok(response) => response,
        Err(failure) => {
            failure.emit();
            failure.into_response()?
        }
    };
    Ok(with_cors_headers(response))
}

/// Why an object request was not carried out.
enum Failure {
    /// The permit is not good for this request.
    Refused(PermitRefusal),
    /// A write declared no length, and the store needs one up front.
    LengthRequired,
    /// A write's body was not as long as it declared.
    LengthMismatch { declared: u64, received: u64 },
    /// The body a write carried does not hash to what the permit binds.
    ChecksumMismatch,
    /// The service could not reach its own configuration or storage.
    Unavailable(String),
}

impl Failure {
    fn status(&self) -> u16 {
        match self {
            Self::Refused(refusal) => refusal.status(),
            Self::LengthRequired => 411,
            Self::LengthMismatch { .. } | Self::ChecksumMismatch => 400,
            Self::Unavailable(_) => 500,
        }
    }

    fn into_response(self) -> Result<Response> {
        let status = self.status();
        let response = match self {
            Self::Refused(refusal) => Response::from_json(&refusal)?,
            Self::LengthRequired => Response::from_json(&serde_json::json!({
                "kind": "LengthRequired",
                "detail": "a write must declare its length",
            }))?,
            Self::LengthMismatch { declared, received } => {
                Response::from_json(&serde_json::json!({
                    "kind": "LengthMismatch",
                    "detail": format!("the body declared {declared} bytes and carried {received}"),
                }))?
            }
            Self::ChecksumMismatch => Response::from_json(&serde_json::json!({
                "kind": "ChecksumMismatch",
                "detail": "the body does not hash to the checksum the permit binds",
            }))?,
            // The reason names internal infrastructure; it goes to
            // the log, not the caller.
            Self::Unavailable(_) => Response::from_json(&serde_json::json!({
                "kind": "Unavailable",
                "detail": "object storage unavailable, retry shortly",
            }))?,
        };
        Ok(response.with_status(status))
    }

    fn emit(&self) {
        let status = self.status();
        let (outcome, kind) = match self {
            Self::Refused(_) => (AccessOutcome::Refused, AccessFailureKind::AccessDenied),
            Self::LengthRequired | Self::LengthMismatch { .. } | Self::ChecksumMismatch => {
                (AccessOutcome::Refused, AccessFailureKind::Invalid)
            }
            Self::Unavailable(reason) => {
                console_error!("object operation failed: {reason}");
                (AccessOutcome::Unavailable, AccessFailureKind::Unavailable)
            }
        };
        AccessFailureLog::new(
            AccessOperation::Object,
            outcome,
            kind,
            status,
            status >= 500,
            AccessSite::Object,
        )
        .emit();
    }
}

impl From<PermitRefusal> for Failure {
    fn from(refusal: PermitRefusal) -> Self {
        Self::Refused(refusal)
    }
}

async fn perform(
    req: &mut Request,
    ctx: &RouteContext<()>,
    method: Method,
) -> std::result::Result<Response, Failure> {
    let key = super::ucan::permit_key(&ctx.env)
        .map_err(|refusal| Failure::Unavailable(format!("{refusal:?}")))?;
    let url = req
        .url()
        .map_err(|error| Failure::Unavailable(format!("request url: {error}")))?;
    let now = Date::now().as_millis() / 1_000;
    let claims = key.verify(method, url.path(), url.query(), now)?;

    let bucket = ctx
        .env
        .bucket("BUCKET")
        .map_err(|error| Failure::Unavailable(format!("Missing BUCKET: {error}")))?;

    match claims.method {
        Method::Get => {
            let range = req
                .headers()
                .get("range")
                .ok()
                .flatten()
                .and_then(|value| parse_range(&value));
            get(&bucket, &claims.key, range).await
        }
        Method::Put => put(req, &bucket, &claims).await,
        Method::Delete => match store::delete(&bucket, &claims).await {
            Ok(true) => Ok(Response::empty()
                .map_err(|error| Failure::Unavailable(error.to_string()))?
                .with_status(204)),
            Ok(false) => precondition_failed(),
            Err(error) => Err(Failure::Unavailable(format!(
                "delete {}: {error}",
                claims.key
            ))),
        },
    }
}

/// Store the request body as the object the claims name, streaming it
/// into the binding.
///
/// The store needs the length up front, so a write must declare one;
/// the body then flows through a fixed-length stream, which fails the
/// write if the client sends more or less than declared. The chunks
/// are hashed as they pass. R2 checks the bound checksum on its side
/// and refuses to store a mismatch, so the hash taken here never
/// decides whether the object lands; it decides what a refused write
/// is answered with, since the store's error says nothing a caller
/// can act on.
async fn put(
    req: &mut Request,
    bucket: &Bucket,
    claims: &Claims,
) -> std::result::Result<Response, Failure> {
    let declared: u64 = req
        .headers()
        .get("content-length")
        .ok()
        .flatten()
        .and_then(|value| value.parse().ok())
        .ok_or(Failure::LengthRequired)?;

    let progress = Rc::new(RefCell::new(Progress::default()));
    let value: JsValue = if declared == 0 {
        // The runtime gives an empty request no body stream at all.
        Uint8Array::new_with_length(0).into()
    } else {
        let stream = req
            .stream()
            .map_err(|error| Failure::Unavailable(format!("request body: {error}")))?;
        let watched = Watched {
            inner: stream,
            progress: progress.clone(),
        };
        Data::Stream(FixedLengthStream::wrap(watched, declared)).into()
    };

    let stored = store::put(bucket, claims, value).await;
    let progress = progress.borrow();
    match stored {
        Ok(Some(etag)) => {
            let headers = Headers::new();
            let _ = headers.set("etag", &etag);
            Ok(Response::empty()
                .map_err(|error| Failure::Unavailable(error.to_string()))?
                .with_headers(headers))
        }
        Ok(None) => precondition_failed(),
        Err(error) => {
            // The store refused, or failed. Which one is read off what
            // the body did on its way in: a body that overran or ran
            // out is the client's, a body of the declared length that
            // does not hash to the bound checksum is the client's, and
            // anything else is the store's.
            let overran = progress.received > declared;
            let ran_out = progress.ended && progress.received < declared;
            if overran || ran_out {
                return Err(Failure::LengthMismatch {
                    declared,
                    received: progress.received,
                });
            }
            let complete = progress.ended && progress.received == declared;
            if complete
                && let Some(expected) = &claims.sha256
                && progress.hasher.clone().finalize()[..] != expected[..]
            {
                return Err(Failure::ChecksumMismatch);
            }
            Err(Failure::Unavailable(format!("put {}: {error}", claims.key)))
        }
    }
}

/// What a request body did on its way into the store.
#[derive(Default)]
struct Progress {
    hasher: Sha256,
    received: u64,
    /// The client finished sending. Without this a store that failed
    /// mid-body would read as a client that stopped short.
    ended: bool,
}

/// A body stream that records its [`Progress`] as it is drained.
struct Watched<S> {
    inner: S,
    progress: Rc<RefCell<Progress>>,
}

impl<S: Stream<Item = Result<Vec<u8>>> + Unpin> Stream for Watched<S> {
    type Item = Result<Vec<u8>>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let polled = Pin::new(&mut self.inner).poll_next(cx);
        match &polled {
            Poll::Ready(Some(Ok(chunk))) => {
                let mut progress = self.progress.borrow_mut();
                progress.hasher.update(chunk);
                progress.received += chunk.len() as u64;
            }
            Poll::Ready(None) => self.progress.borrow_mut().ended = true,
            Poll::Ready(Some(Err(_))) | Poll::Pending => {}
        }
        polled
    }
}

fn precondition_failed() -> std::result::Result<Response, Failure> {
    Response::empty()
        .map(|response| response.with_status(412))
        .map_err(|error| Failure::Unavailable(error.to_string()))
}

/// Read `key`, streaming the body straight from the binding to the
/// client so the worker spends no CPU time on the bytes.
async fn get(
    bucket: &Bucket,
    key: &str,
    range: Option<Range>,
) -> std::result::Result<Response, Failure> {
    let mut request = bucket.get(key);
    if let Some(range) = range.clone() {
        request = request.range(range);
    }
    let object = request
        .execute()
        .await
        .map_err(|error| Failure::Unavailable(format!("get {key}: {error}")))?;
    let Some(object) = object else {
        return Response::empty()
            .map(|response| response.with_status(404))
            .map_err(|error| Failure::Unavailable(error.to_string()));
    };

    let headers = Headers::new();
    // Whatever content type the object was stored with; a block or a
    // cell has none, and answers as bytes.
    object
        .write_http_metadata(headers.clone())
        .map_err(|error| Failure::Unavailable(format!("get {key}: {error}")))?;
    if !headers.has("content-type").unwrap_or(false) {
        let _ = headers.set("content-type", "application/octet-stream");
    }
    let _ = headers.set("etag", &object.http_etag());
    let _ = headers.set("accept-ranges", "bytes");
    let size = object.size();
    let (status, length) = match range {
        None => (200, size),
        Some(_) => {
            let served = object
                .range()
                .map_err(|error| Failure::Unavailable(format!("get {key}: {error}")))?;
            let (start, end) = match served {
                Range::OffsetWithLength { offset, length } => (offset, offset + length - 1),
                Range::OffsetToEnd { offset } => (offset, size.saturating_sub(1)),
                Range::Prefix { length } => (0, length.saturating_sub(1)),
                Range::Suffix { suffix } => (size.saturating_sub(suffix), size.saturating_sub(1)),
            };
            let _ = headers.set("content-range", &format!("bytes {start}-{end}/{size}"));
            (206, end - start + 1)
        }
    };
    let _ = headers.set("content-length", &length.to_string());

    let body = object
        .body()
        .ok_or_else(|| Failure::Unavailable(format!("get {key}: the object came without a body")))?
        .response_body()
        .map_err(|error| Failure::Unavailable(format!("get {key}: {error}")))?;
    Response::from_body(body)
        .map(|response| response.with_status(status).with_headers(headers))
        .map_err(|error| Failure::Unavailable(error.to_string()))
}

/// The single byte range a `Range` header asks for, in the binding's
/// terms. `None` for anything else — a malformed header, or several
/// ranges — which serves the whole object, as the specification allows.
pub(crate) fn parse_range(header: &str) -> Option<Range> {
    let spec = header.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    match (start.trim(), end.trim()) {
        ("", suffix) => Some(Range::Suffix {
            suffix: suffix.parse().ok()?,
        }),
        (start, "") => Some(Range::OffsetToEnd {
            offset: start.parse().ok()?,
        }),
        (start, end) => {
            let offset: u64 = start.parse().ok()?;
            let end: u64 = end.parse().ok()?;
            (end >= offset).then(|| Range::OffsetWithLength {
                offset,
                length: end - offset + 1,
            })
        }
    }
}

/// The writes, shared with enrollment's custody-cell write.
pub mod store {
    use super::{Claims, Precondition};
    use worker::js_sys::{Object as JsObject, Reflect, Uint8Array};
    use worker::wasm_bindgen::{JsCast, JsValue};
    use worker::wasm_bindgen_futures::JsFuture;
    use worker::{Bucket, Headers, Result};

    /// Store `value` — bytes, or a stream of known length — as the
    /// object the claims name, under the checksum and precondition
    /// they bind. `None` when the precondition did not hold; otherwise
    /// the stored object's `ETag`. A body that does not hash to the
    /// bound checksum is refused by the store and comes back as an
    /// error.
    ///
    /// Goes to the binding directly rather than through the builder,
    /// which spells conditions as a bare entity tag and has no way to
    /// say "only if absent". A `Headers` condition carries every
    /// conditional header the S3 endpoint accepted, `If-None-Match: *`
    /// included, so the write keeps the exact semantics the permit
    /// was issued with.
    pub async fn put(bucket: &Bucket, claims: &Claims, value: JsValue) -> Result<Option<String>> {
        let options = JsObject::new();
        match &claims.precondition {
            Precondition::None => {}
            condition => {
                let headers = Headers::new();
                match condition {
                    Precondition::IfMatch(etag) => {
                        headers.set("if-match", &format!("\"{etag}\""))?;
                    }
                    Precondition::IfNoneMatch => headers.set("if-none-match", "*")?,
                    Precondition::None => {}
                }
                Reflect::set(&options, &"onlyIf".into(), &headers.0.into())?;
            }
        }
        if let Some(digest) = &claims.sha256 {
            let digest = Uint8Array::from(digest.as_slice());
            Reflect::set(&options, &"sha256".into(), &digest.buffer().into())?;
        }
        let inner: &worker::worker_sys::R2Bucket = bucket.as_ref().unchecked_ref();
        let stored = JsFuture::from(inner.put(claims.key.clone(), value, options.into())?).await?;
        if stored.is_null() {
            return Ok(None);
        }
        let object: worker::worker_sys::R2Object = stored.unchecked_into();
        Ok(Some(object.http_etag()?))
    }

    /// Remove the object the claims name, provided their precondition
    /// holds. `false` when it did not.
    ///
    /// The binding has no conditional delete, so the condition is
    /// checked against the object's current version first. The two
    /// steps are not one operation; a write landing between them is
    /// deleted as though it had been the version checked.
    pub async fn delete(bucket: &Bucket, claims: &Claims) -> Result<bool> {
        if !matches!(claims.precondition, Precondition::None) {
            let current = bucket.head(claims.key.clone()).await?;
            let holds = match &claims.precondition {
                Precondition::None => true,
                Precondition::IfMatch(etag) => current
                    .as_ref()
                    .is_some_and(|object| object.etag() == *etag),
                Precondition::IfNoneMatch => current.is_none(),
            };
            if !holds {
                return Ok(false);
            }
        }
        bucket.delete(claims.key.clone()).await?;
        Ok(true)
    }
}
