//! Bounded Worker HTTP adapter for public connection delivery.
use worker::{Request, Response, RouteContext};

fn cors(response: Response) -> Response {
    let headers = response.headers().clone();
    let _ = headers.set("Access-Control-Allow-Origin", "*");
    let _ = headers.set("Access-Control-Allow-Methods", "POST, OPTIONS");
    let _ = headers.set("Access-Control-Allow-Headers", "Content-Type");
    let _ = headers.set("Cache-Control", "no-store");
    response.with_headers(headers)
}

pub async fn options(_req: Request, _ctx: RouteContext<()>) -> worker::Result<Response> {
    Ok(cors(Response::empty()?.with_status(204)))
}

pub async fn handle(mut req: Request, ctx: RouteContext<()>) -> worker::Result<Response> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (&mut req, ctx);
        Ok(cors(Response::empty()?.with_status(501)))
    }
    #[cfg(target_arch = "wasm32")]
    {
        use crate::delivery::{DeliveryResponse, MAX_APPROVAL_BYTES, MAX_READ_BYTES, READ_PATH};
        use dialog_varsig::Principal;
        use futures_util::StreamExt;
        let path = req.path();
        let limit = if path == READ_PATH || path == crate::delivery::additions::READ_ADDITIONS_PATH
        {
            MAX_READ_BYTES
        } else {
            MAX_APPROVAL_BYTES
        };
        let mut stream = req.stream()?;
        let mut body = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if chunk.len() > limit.saturating_sub(body.len()) {
                return response(DeliveryResponse::error(413, "delivery body too large"));
            }
            body.extend_from_slice(&chunk);
        }
        let store = crate::store::d1::D1Store::new(ctx.env.d1("CONTROL")?);
        let index =
            crate::revocation::index::kv::KvRevocationIndex::new(ctx.env.kv("REVOCATIONS_KV")?);
        let key = ctx.env.secret("SERVICE_SECRET_KEY")?.to_string();
        let signer = crate::service::signer_from_hex(&key).map_err(worker::Error::RustError)?;
        let now = (js_sys::Date::now() / 1000.0) as u64;
        response(crate::delivery::execute(&store, &index, &signer.did(), &path, &body, now).await)
    }
}

#[cfg(target_arch = "wasm32")]
fn response(result: crate::delivery::DeliveryResponse) -> worker::Result<Response> {
    let response = Response::from_bytes(result.body)?.with_status(result.status);
    let headers = response.headers().clone();
    headers.set("Content-Type", result.content_type)?;
    Ok(cors(response.with_headers(headers)))
}
