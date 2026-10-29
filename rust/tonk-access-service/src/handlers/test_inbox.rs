//! The preview's stand-in for an inbox, so the hosted agent's end to end
//! test (`tests/e2e/hosted-agent`) can sign up a fresh account in CI
//! without reading real mail.
//!
//! - `GET /_test/emails?address=<email>` — `[[email, link]]` for the
//!   latest activation link mailed to `email`, or `[]`. The same shape as
//!   the helpers server's `/_test/emails`, filtered to one address.
//!
//! It exists only where the `TEST_INBOX` KV namespace is bound, which is
//! only the per-pull-request preview; everywhere else the route answers
//! 404 and every send goes out as mail. Even there it holds only mail to
//! addresses under `.test`, a top-level name reserved never to resolve
//! (RFC 2606), so no real person's activation link ever lands here.

use worker::{Request, Response, RouteContext};

/// The KV binding whose presence turns the inbox on.
pub const BINDING: &str = "TEST_INBOX";

/// How long a recorded link is kept, in seconds. A run reads it within
/// a minute of sign-up.
#[cfg(target_arch = "wasm32")]
const TTL: u64 = 60 * 60;

/// Whether mail to `email` is recorded rather than sent.
pub fn reserved(email: &str) -> bool {
    email
        .rsplit_once('@')
        .is_some_and(|(_, domain)| domain.to_ascii_lowercase().ends_with(".test"))
}

#[cfg(target_arch = "wasm32")]
fn key(email: &str) -> String {
    format!("activation:{}", crate::email::normalize_email(email))
}

/// The inbox, when this deployment binds one.
#[cfg(target_arch = "wasm32")]
pub fn bound(env: &worker::Env) -> Option<worker::kv::KvStore> {
    env.kv(BINDING).ok()
}

/// Record `link` as the latest activation mailed to `email`.
#[cfg(target_arch = "wasm32")]
pub async fn record(
    inbox: &worker::kv::KvStore,
    email: &str,
    link: &str,
) -> Result<(), crate::email::EmailError> {
    let failed = |err: worker::kv::KvError| crate::email::EmailError::Send(err.to_string());
    inbox
        .put(&key(email), link)
        .map_err(failed)?
        .expiration_ttl(TTL)
        .execute()
        .await
        .map_err(failed)
}

/// GET `/_test/emails` (native build: there is no KV, so no inbox).
#[cfg(not(target_arch = "wasm32"))]
pub async fn handle(_req: Request, _ctx: RouteContext<()>) -> worker::Result<Response> {
    Response::error("Not Found", 404)
}

/// GET `/_test/emails?address=<email>`.
#[cfg(target_arch = "wasm32")]
pub async fn handle(req: Request, ctx: RouteContext<()>) -> worker::Result<Response> {
    let Some(inbox) = bound(&ctx.env) else {
        return Response::error("Not Found", 404);
    };
    let url = req.url()?;
    let Some(address) = url
        .query_pairs()
        .find(|(name, _)| name == "address")
        .map(|(_, value)| value.into_owned())
    else {
        return Response::error("address is required", 400);
    };
    if !reserved(&address) {
        return Response::error("only .test addresses are recorded", 400);
    }
    let link = inbox
        .get(&key(&address))
        .text()
        .await
        .map_err(|err| worker::Error::RustError(err.to_string()))?;
    let found: Vec<(String, String)> = link.map(|link| (address, link)).into_iter().collect();
    let response = Response::from_json(&found)?;
    response.headers().set("Cache-Control", "no-store")?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::reserved;

    #[test]
    fn only_reserved_test_addresses_are_recorded() {
        assert!(reserved("e2e-1@hosted-agent.test"));
        assert!(reserved("E2E@Example.TEST"));
        assert!(!reserved("someone@example.com"));
        assert!(!reserved("someone@test.example.com"));
        assert!(!reserved("no-at-sign.test"));
    }
}
