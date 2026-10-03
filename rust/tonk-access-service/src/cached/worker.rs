//! The Workers Cache API as an [`ObjectCache`].
//!
//! Objects are keyed by a URL on a host that does not exist, since the
//! Cache API keys by URL and these objects have none of their own. A
//! range is asked for the way HTTP asks for one, and the Cache API
//! answers it out of the whole it holds.

use dialog_effects::blob::{BlobReader, ByteRange};
use worker::{Cache, Headers, Request, RequestInit, Response};

use super::ObjectCache;
use crate::objects::Streamed;

/// Where cached objects are keyed. The host is never resolved.
const ORIGIN: &str = "https://objects.tonk.invalid/";

/// Content-addressed objects never change: cache them for as long as
/// the cache will keep them.
const CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

/// The data center's cache.
pub struct WorkerCache {
    cache: Cache,
}

impl Default for WorkerCache {
    fn default() -> Self {
        Self {
            cache: Cache::default(),
        }
    }
}

fn url(key: &str) -> String {
    format!("{ORIGIN}{key}")
}

#[async_trait::async_trait(?Send)]
impl ObjectCache for WorkerCache {
    async fn read(&self, key: &str, range: Option<ByteRange>) -> Option<BlobReader> {
        let url = url(key);
        let mut response = match range {
            None => self.cache.get(url.as_str(), false).await.ok()??,
            Some(range) => {
                let headers = Headers::new();
                let value = match range.length {
                    Some(length) => format!(
                        "bytes={}-{}",
                        range.offset,
                        range.offset + length.max(1) - 1
                    ),
                    None => format!("bytes={}-", range.offset),
                };
                headers.set("Range", &value).ok()?;
                let mut init = RequestInit::new();
                init.with_headers(headers);
                let request = Request::new_with_init(&url, &init).ok()?;
                let response = self.cache.get(&request, false).await.ok()??;
                // Only a range answer is the range; a whole object in
                // its place would be served as if it were one.
                if response.status_code() != 206 {
                    return None;
                }
                response
            }
        };
        let stream = response.stream().ok()?;
        Some(Box::new(Streamed::new(stream)))
    }

    async fn write(&self, key: &str, bytes: Vec<u8>) {
        let headers = Headers::new();
        let _ = headers.set("Cache-Control", CACHE_CONTROL);
        let _ = headers.set("Content-Type", "application/octet-stream");
        let _ = headers.set("Content-Length", &bytes.len().to_string());
        if let Ok(response) = Response::from_bytes(bytes) {
            let _ = self
                .cache
                .put(url(key).as_str(), response.with_headers(headers))
                .await;
        }
    }
}
