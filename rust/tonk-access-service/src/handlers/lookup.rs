//! Worker-side glue for the email lookup.
//!
//! The logic lives in [`crate::lookup`], generic over storage; this
//! module binds it to D1 and shapes the HTTP answer. See that module for
//! the DID form and why the address is encoded rather than hashed.

use worker::{Request, Response, RouteContext};

/// The `Cache-Control` a settled answer carries.
///
/// A customer's registration state changes, so even a settled answer is
/// cacheable only briefly: a minute keeps a burst of lookups off D1
/// without letting a suspension go unnoticed for long.
#[cfg(target_arch = "wasm32")]
const CACHE_CONTROL: &str = "public, max-age=60";

/// The `Cache-Control` an unsettled answer carries.
///
/// A `202` says the customer has not confirmed their address yet, and a
/// `404` that nobody has claimed it. Both are about to change, and the
/// change is what the caller is waiting on: an invite flow polling an
/// address must not be told for a minute that the person it just invited
/// is still unregistered. Storing nothing our own side is not enough,
/// because this header is what browsers and intermediaries obey.
#[cfg(target_arch = "wasm32")]
const NO_CACHE: &str = "no-store";

/// The rate limit binding, declared in `wrangler.toml`.
#[cfg(target_arch = "wasm32")]
const LIMITER: &str = "LOOKUP_LIMITER";

/// What a throttled caller is told to wait, matching the binding's
/// period. The limiter is a fixed window, so this is an upper bound.
#[cfg(target_arch = "wasm32")]
const RETRY_AFTER: &str = "60";

/// GET `/customer/:domain/:local/did.json` → the DID document for an
/// email address.
///
/// `404` when nothing is registered under the address, or when the path
/// segments do not form one: an address nobody registered and an address
/// that cannot exist are the same fact to a caller.
#[cfg(not(target_arch = "wasm32"))]
pub async fn handle(_req: Request, _ctx: RouteContext<()>) -> worker::Result<Response> {
    Response::error("Not Found", 404)
}

/// GET `/customer/:domain/:local/did.json` (worker body; see the native
/// twin above, and the helpers server for the native implementation).
#[cfg(target_arch = "wasm32")]
pub async fn handle(req: Request, ctx: RouteContext<()>) -> worker::Result<Response> {
    use crate::lookup::{address_from_segments, customer_did, resolve};
    use crate::store::d1::D1Store;
    use worker::Cache;

    // A cached answer skips both the limiter and D1. That is the right
    // order: repeat lookups of one address are absorbed here, so what
    // reaches the limiter below is closer to the distinct-address rate,
    // which is the signal enumeration actually produces.
    let cache = Cache::default();
    if let Ok(Some(hit)) = cache.get(&req, false).await {
        return Ok(hit);
    }

    // Keyed by caller, not by address: a budget that reset per address
    // would not constrain walking a list of them, which is the thing
    // worth limiting on an endpoint whose URL carries the address.
    if let Ok(limiter) = ctx.env.rate_limiter(LIMITER) {
        let caller = req
            .headers()
            .get("CF-Connecting-IP")
            .ok()
            .flatten()
            .unwrap_or_default();
        match limiter.limit(caller).await {
            Ok(outcome) if !outcome.success => return throttled(),
            Ok(_) => {}
            // A limiter that cannot answer must not take the endpoint
            // down with it; the lookup discloses nothing a caller could
            // not reach through the registration probe anyway.
            Err(err) => worker::console_error!("lookup rate limit unavailable: {err}"),
        }
    }

    // Segments are read raw. `did:web` resolution already percent-decoded
    // them, and the router hands them over untouched, so a `+` in a local
    // part stays a `+` rather than becoming a space.
    let (Some(domain), Some(local)) = (ctx.param("domain"), ctx.param("local")) else {
        return not_found();
    };
    let Some(address) = address_from_segments(domain, local) else {
        return not_found();
    };

    let store = match ctx.env.d1("CONTROL") {
        Ok(database) => D1Store::new(database),
        Err(err) => {
            worker::console_error!("email lookup unavailable, no CONTROL binding: {err}");
            return with_cors(Response::error("Customer registry is not configured", 500));
        }
    };

    let url = req.url()?;
    let host = url.host_str().map(ToString::to_string).unwrap_or_default();
    let origin = url.origin().ascii_serialization();
    let Some(did) = customer_did(&host, &address) else {
        return not_found();
    };

    match resolve(&store, &did, &address, &origin).await {
        Ok(Some(found)) => {
            let body = serde_json::to_string_pretty(&found.document)
                .map_err(|error| worker::Error::RustError(error.to_string()))?;
            let response = Response::ok(body)?.with_status(found.status).with_headers({
                let headers = worker::Headers::new();
                let _ = headers.set("content-type", "application/json");
                headers
            });
            let mut response = with_cors(Ok(response))?;
            // Only a settled answer is cacheable, by us or by anyone
            // downstream. A `202` is by definition about to change, and
            // holding it would leave an activated customer reading as
            // unconfirmed until the entry aged out.
            let settled = found.status == 200;
            response.headers_mut().set(
                "Cache-Control",
                if settled { CACHE_CONTROL } else { NO_CACHE },
            )?;
            if settled {
                let stored = response.cloned()?;
                cache.put(&req, stored).await?;
            }
            Ok(response)
        }
        Ok(None) => not_found(),
        Err(err) => {
            worker::console_error!("email lookup failed: {err}");
            with_cors(Response::error("Customer registry is unavailable", 500))
        }
    }
}

/// The answer for a caller over their budget.
#[cfg(target_arch = "wasm32")]
fn throttled() -> worker::Result<Response> {
    let mut response = with_cors(
        Response::from_json(&serde_json::json!({ "error": "too many lookups" }))
            .map(|response| response.with_status(429)),
    )?;
    response.headers_mut().set("Retry-After", RETRY_AFTER)?;
    Ok(response)
}

/// The answer for an address no customer holds, and for a path that does
/// not form an address.
#[cfg(target_arch = "wasm32")]
fn not_found() -> worker::Result<Response> {
    let mut response = with_cors(
        Response::from_json(&serde_json::json!({ "error": "no customer for this address" }))
            .map(|response| response.with_status(404)),
    )?;
    response.headers_mut().set("Cache-Control", NO_CACHE)?;
    Ok(response)
}

/// Attach the permissive CORS header every public route on this service
/// answers with; the invite flow calls this one from the browser.
#[cfg(target_arch = "wasm32")]
fn with_cors(response: worker::Result<Response>) -> worker::Result<Response> {
    let mut response = response?;
    response
        .headers_mut()
        .set("Access-Control-Allow-Origin", "*")?;
    Ok(response)
}
