//! The requests and the answers of `wasi:http/handler`, for a guest kernel that
//! serves the routes of its domain. See PDD007, "The World": each request that
//! a client sends on a handed-over socket becomes one call to `handle`.

use wit_bindgen::{FutureReader, StreamResult, StreamWriter};

use crate::world::wasi::http::types::{ErrorCode, Fields, Method, Request, Response};
use crate::world::{wit_future, wit_stream};

/// `method`, as an HTTP method.
pub fn method(method: &Method) -> axum::http::Method {
    match method {
        Method::Get => axum::http::Method::GET,
        Method::Head => axum::http::Method::HEAD,
        Method::Post => axum::http::Method::POST,
        Method::Put => axum::http::Method::PUT,
        Method::Delete => axum::http::Method::DELETE,
        Method::Connect => axum::http::Method::CONNECT,
        Method::Options => axum::http::Method::OPTIONS,
        Method::Trace => axum::http::Method::TRACE,
        Method::Patch => axum::http::Method::PATCH,
        Method::Other(other) => {
            axum::http::Method::from_bytes(other.as_bytes()).unwrap_or(axum::http::Method::GET)
        }
    }
}

/// The whole body of `request`.
pub async fn read_body(request: Request) -> Result<Vec<u8>, ErrorCode> {
    Ok(read_body_within(request, usize::MAX)
        .await?
        .expect("no body is over usize::MAX bytes"))
}

/// The whole body of `request`, or `None` once it is over `limit` bytes: the
/// read then stops, and the rest of the body is left unread.
pub async fn read_body_within(
    request: Request,
    limit: usize,
) -> Result<Option<Vec<u8>>, ErrorCode> {
    let (_, done) = wit_future::new(|| Ok(()));
    let (mut contents, trailers) = Request::consume_body(request, done);
    let mut body = Vec::new();
    let mut chunk = Vec::with_capacity(4096);
    loop {
        let (status, buffer) = contents.read(chunk).await;
        chunk = buffer;
        match status {
            StreamResult::Complete(_) => {
                body.extend_from_slice(&chunk);
                chunk.clear();
                if body.len() > limit {
                    return Ok(None);
                }
            }
            StreamResult::Dropped | StreamResult::Cancelled => break,
        }
    }
    trailers.await?;
    Ok(Some(body))
}

/// An answer with `status`, and `body` of the type `content_type`.
pub fn respond(status: u16, content_type: Option<&str>, body: Vec<u8>) -> Response {
    let headers = content_type
        .map(|content_type| vec![("content-type".to_owned(), content_type.as_bytes().to_vec())])
        .unwrap_or_default();
    let (response, mut writer, _) = stream(status, &headers);
    wit_bindgen::spawn_local(async move {
        let rest = writer.write_all(body).await;
        debug_assert!(rest.is_empty(), "the client took the whole body");
    });
    response
}

/// The end of the body of an answer: it resolves once the embedder sent the
/// whole body, or once the client went away before.
pub type Sent = FutureReader<Result<(), ErrorCode>>;

/// An answer with `status` and `headers`, whose body the guest kernel writes
/// to the writer as it goes, such as the events of a subscription, and the
/// end of its body. The body ends when the writer drops.
pub fn stream(status: u16, headers: &[(String, Vec<u8>)]) -> (Response, StreamWriter<u8>, Sent) {
    let headers = Fields::from_list(headers).unwrap_or_else(|_| Fields::new());
    let (writer, reader) = wit_stream::new();
    let (_, trailers) = wit_future::new(|| Ok(None));
    let (response, sent) = Response::new(headers, Some(reader), trailers);
    let _ = response.set_status_code(status);
    (response, writer, sent)
}
