//! Bounded HTTP adapter for the shared revocation-index lookup.

/// Native requests are handled by `helpers::server`, not the worker router.
#[cfg(not(target_arch = "wasm32"))]
pub async fn handle(
    _: worker::Request,
    _: worker::RouteContext<()>,
) -> worker::Result<worker::Response> {
    worker::Response::error("use the native service adapter", 503)
}

#[cfg(target_arch = "wasm32")]
use worker::{Request, Response, RouteContext};

/// POST /ucan/revocations: no grants, side effects, or registry enumeration.
#[cfg(target_arch = "wasm32")]
pub async fn handle(mut req: Request, ctx: RouteContext<()>) -> worker::Result<Response> {
    use crate::revocation::{index::kv::KvRevocationIndex, lookup};
    use futures_util::StreamExt;
    use tonk_identity::revocation::evidence::MAX_BODY_BYTES;

    let outcome = async {
        let mut body = Vec::new();
        let mut stream = req
            .stream()
            .map_err(|error| lookup::Error::Invalid(error.to_string()))?;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| lookup::Error::Invalid(error.to_string()))?;
            if body.len().saturating_add(chunk.len()) > MAX_BODY_BYTES {
                return Err(lookup::Error::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        let index = KvRevocationIndex::new(
            ctx.env
                .kv("REVOCATIONS_KV")
                .map_err(|_| lookup::Error::Unavailable)?,
        );
        lookup::answer(&index, &body).await
    }
    .await;
    let response = match outcome {
        Ok(answer) => Response::from_json(&answer)?,
        Err(error) => Response::from_json(&serde_json::json!({"error": error.to_string()}))?
            .with_status(error.status()),
    };
    let headers = response.headers().clone();
    headers.set("Access-Control-Allow-Origin", "*")?;
    headers.set("Cache-Control", "no-store")?;
    Ok(response.with_headers(headers))
}
