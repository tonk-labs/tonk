//! Streaming storage adapter for a verified, request-local operation.
use crate::operation::{Method, Operation, Precondition};
use futures_util::StreamExt;
use worker::js_sys::Uint8Array;
use worker::{
    Bucket, Data, FixedLengthStream, Headers, Range, Request, Response, Result, RouteContext,
};

fn with_cors(response: Response) -> Response {
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
pub async fn handle_options(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Ok(with_cors(Response::empty()?.with_status(204)))
}
/// Old bearer URLs cannot prove present authority. Keep typed 401 so cached
/// readers redeem again; an in-flight old mutation must be retried by its caller.
pub async fn handle_get(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    retired()
}
pub async fn handle_put(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    retired()
}
pub async fn handle_delete(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    retired()
}
fn retired() -> Result<Response> {
    Ok(with_cors(
        Response::from_json(&crate::operation::reauthorization())?.with_status(401),
    ))
}

pub(crate) enum Failure {
    LengthRequired,
    Invalid(String),
    Unavailable(String),
}
impl Failure {
    pub(crate) fn into_response(self) -> Result<Response> {
        let (status, kind, detail) = match self {
            Self::LengthRequired => (
                411,
                "LengthRequired",
                "a write must declare its length".to_string(),
            ),
            Self::Invalid(detail) => (400, "InvalidPayload", detail),
            Self::Unavailable(detail) => {
                worker::console_error!("object storage: {detail}");
                (
                    503,
                    "Unavailable",
                    "object storage unavailable, retry shortly".into(),
                )
            }
        };
        Ok(
            Response::from_json(&serde_json::json!({"kind":kind,"detail":detail}))?
                .with_status(status),
        )
    }
}
pub(crate) async fn execute(
    req: &mut Request,
    bucket: &Bucket,
    operation: &Operation,
    upload_bytes: &mut u64,
) -> std::result::Result<Response, Failure> {
    match operation.method {
        Method::Get => {
            get(
                bucket,
                &operation.key,
                operation.range.map(|r| match r.length {
                    Some(length) => Range::OffsetWithLength {
                        offset: r.offset,
                        length,
                    },
                    None => Range::OffsetToEnd { offset: r.offset },
                }),
            )
            .await
        }
        Method::Put => put(req, bucket, operation, upload_bytes).await,
        Method::Delete => match store::delete(bucket, operation).await {
            Ok(true) => Response::empty()
                .map(|r| r.with_status(204))
                .map_err(|e| Failure::Unavailable(e.to_string())),
            Ok(false) => precondition_failed(),
            Err(e) => Err(Failure::Unavailable(e.to_string())),
        },
    }
}
async fn put(
    req: &mut Request,
    bucket: &Bucket,
    operation: &Operation,
    upload_bytes: &mut u64,
) -> std::result::Result<Response, Failure> {
    let declared = req
        .headers()
        .get("content-length")
        .ok()
        .flatten()
        .and_then(|v| v.parse::<u64>().ok())
        .or(operation.size);
    // Preserve the prior direct transport's unknown-length archive/cell
    // collection. This fallback intentionally retains its unbounded memory risk.
    let buffered = if declared.is_none() {
        let bytes = req
            .bytes()
            .await
            .map_err(|e| Failure::Invalid(e.to_string()))?;
        if bytes.is_empty() {
            return Err(Failure::LengthRequired);
        }
        crate::upload::validate_buffer(operation, &bytes).map_err(Failure::Invalid)?;
        Some(bytes)
    } else {
        None
    };
    let declared =
        declared.unwrap_or_else(|| buffered.as_ref().expect("collected upload").len() as u64);
    *upload_bytes = declared;
    if operation.size.is_some_and(|n| n != declared) {
        return Err(Failure::Invalid(
            "Content-Length differs from signed size".into(),
        ));
    }
    let (value, progress) = if let Some(bytes) = buffered {
        (Uint8Array::from(bytes.as_slice()).into(), None)
    } else if declared == 0 {
        crate::upload::validate_empty(operation).map_err(Failure::Invalid)?;
        if let Ok(mut stream) = req.stream() {
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| Failure::Invalid(e.to_string()))?;
                if !chunk.is_empty() {
                    return Err(Failure::Invalid(
                        "nonempty body for zero-sized upload".into(),
                    ));
                }
            }
        }
        (Uint8Array::new_with_length(0).into(), None)
    } else {
        let input = req
            .stream()
            .map_err(|e| Failure::Invalid(e.to_string()))?
            .map(|item| item.map_err(|e| e.to_string()));
        let (validated, progress) =
            crate::upload::Validated::new(input, operation.clone(), declared);
        let stream = validated.map(|item| item.map_err(worker::Error::RustError));
        (
            Data::Stream(FixedLengthStream::wrap(stream, declared)).into(),
            Some(progress),
        )
    };
    let stored = store::put(bucket, operation, value).await;
    if let Some(progress) = progress
        && let crate::upload::Verdict::Invalid(detail) = &*progress.lock().expect("upload progress")
    {
        return Err(Failure::Invalid(detail.clone()));
    }
    match stored {
        Ok(Some(etag)) => {
            let headers = Headers::new();
            let _ = headers.set("etag", &etag);
            Response::empty()
                .map(|r| r.with_headers(headers))
                .map_err(|e| Failure::Unavailable(e.to_string()))
        }
        Ok(None) => precondition_failed(),
        Err(e) => Err(Failure::Unavailable(e.to_string())),
    }
}
fn precondition_failed() -> std::result::Result<Response, Failure> {
    Response::empty()
        .map(|response| response.with_status(412))
        .map_err(|error| Failure::Unavailable(error.to_string()))
}

/// Read `key`, streaming the body straight from the binding to the
/// client so the worker spends no CPU time on the bytes.
pub(crate) async fn get(
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
#[cfg(all(feature = "helpers", not(target_arch = "wasm32")))]
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
    use super::{Operation, Precondition};
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
    pub async fn put(
        bucket: &Bucket,
        claims: &Operation,
        value: JsValue,
    ) -> Result<Option<String>> {
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
    pub async fn delete(bucket: &Bucket, claims: &Operation) -> Result<bool> {
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
