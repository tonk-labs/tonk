//! `account/check-email` — is this address already registered?
//!
//! The registration form asks as the user types, and routes on the
//! answer: create an account, sign in, or say why neither is on offer.
//! Asking first is what keeps someone from running a whole WebAuthn
//! creation ceremony against an address that already has an account and
//! failing at the end, leaving an orphan passkey in the authenticator.
//!
//! The answer is written to the profile OVERLAY as
//! [`tonk_schema::EmailStatus`], not returned in a response body and not
//! committed to a branch. Two reasons, and both matter:
//!
//! - A command answers with facts the page already subscribes to. The
//!   form renders from the row; nothing reads a result.
//! - The form asks per keystroke. A durable row per answer would write
//!   one per character into a branch that syncs.
//!
//! The row carries the address alongside the state, so a form that has
//! moved on can tell an answer about what is typed now from an answer
//! about what was typed two characters ago.
//!
//! Format validation is NOT here. `wa-input` does native constraint
//! validation in the component, which is where a question with no
//! network answer belongs; this only refuses what it cannot turn into a
//! lookup path.

/// The states an answer can take, as the form reads them.
///
/// The first four mirror what the access service's lookup answers
/// (`lookup::status_of`): 404 nothing registered, 200 active, 202
/// enrolled but unconfirmed, 410 suspended. The last two are this
/// worker's own: an address that cannot be looked up, and a service that
/// could not be reached.
///
/// `Unavailable` is deliberately not folded into `Unregistered`. A form
/// that read "could not reach the service" as "nobody has this address"
/// would send someone into a creation ceremony that fails at the end.
pub(crate) mod state {
    /// Nothing is registered under the address: offer to create.
    pub(crate) const UNREGISTERED: &str = "unregistered";
    /// Registered and served: offer to sign in.
    pub(crate) const ACTIVE: &str = "active";
    /// Enrolled, activation link unopened: sign in, then wait.
    pub(crate) const PENDING: &str = "pending";
    /// Service withdrawn. Neither creating nor signing in helps.
    pub(crate) const SUSPENDED: &str = "suspended";
    /// Not an address this can look up.
    pub(crate) const INVALID: &str = "invalid";
    /// The service could not be reached, so this says nothing about the
    /// address itself.
    pub(crate) const UNAVAILABLE: &str = "unavailable";
}

/// Split an address into the `(domain, local)` pair the lookup path
/// names, or `None` when it is not one.
///
/// The lookup route is `/customer/{domain}/{local}/did.json`, domain
/// first, matching `did:mailto`. A local part containing `/` would split
/// the route and one containing `@` is ambiguous, so both are refused
/// here rather than sent to produce a confusing 404.
pub(crate) fn split_address(email: &str) -> Option<(String, String)> {
    let trimmed = email.trim();
    if trimmed.len() > 254 {
        return None;
    }
    let (local, domain) = trimmed.rsplit_once('@')?;
    if local.is_empty() || domain.is_empty() || domain.contains('@') || local.contains('/') {
        return None;
    }
    Some((domain.to_ascii_lowercase(), local.to_ascii_lowercase()))
}

/// The state for an address the lookup cannot be asked about.
///
/// Split out from [`lookup`] so the mapping is testable without a
/// service: an address that does not split into a lookup path never
/// leaves this worker.
pub(crate) fn state_for_address(email: &str) -> Option<&'static str> {
    split_address(email).is_none().then_some(state::INVALID)
}

/// Map the lookup's HTTP status onto the state the form reads.
///
/// Pinned to `lookup::status_of` at the other end by
/// [`tests::it_reads_every_status_the_lookup_answers`].
pub(crate) fn state_for_status(status: u16) -> &'static str {
    match status {
        200 => state::ACTIVE,
        202 => state::PENDING,
        404 => state::UNREGISTERED,
        410 => state::SUSPENDED,
        // Anything else is the service failing to answer rather than an
        // answer about the address: a 429 from the lookup's rate limit,
        // a 5xx, a proxy error.
        _ => state::UNAVAILABLE,
    }
}

/// Runs `account/check-email`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) struct CheckEmailHandler {
    attributes: Vec<String>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl CheckEmailHandler {
    pub(crate) fn new() -> Self {
        use crate::reactor::Decode as _;
        Self {
            attributes: tonk_schema::command::CheckEmail::trigger_attributes(),
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn decode_email(facts: &crate::reactor::EntityFacts) -> Option<String> {
    use crate::reactor::Decode as _;
    facts
        .first()
        .map(|artifact| artifact.of.clone())
        .and_then(|entity| tonk_schema::command::CheckEmail::decode(entity, facts))
        .map(|command| command.email.0)
        .filter(|email| !email.trim().is_empty())
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl crate::reactor::CommandHandler<crate::router::CommandEnv> for CheckEmailHandler {
    fn trigger_attributes(&self) -> &[String] {
        &self.attributes
    }

    fn matches(&self, facts: &crate::reactor::EntityFacts) -> bool {
        decode_email(facts).is_some()
    }

    fn run(
        &self,
        facts: &crate::reactor::EntityFacts,
        env: &crate::router::CommandEnv,
    ) -> crate::reactor::RunFuture {
        let email = decode_email(facts);
        let env = env.clone();

        Box::pin(async move {
            let Some(email) = email else {
                return;
            };
            let state = lookup(&email).await;
            publish(&env, &email, state).await;
        })
    }
}

/// Ask the access service about `email`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn lookup(email: &str) -> &'static str {
    use super::http::{HttpError, get};
    use tonk_common::log;

    if let Some(state) = state_for_address(email) {
        return state;
    }
    let Some((domain, local)) = split_address(email) else {
        return state::INVALID;
    };
    let Some(origin) = super::repository::worker_origin() else {
        return state::UNAVAILABLE;
    };
    let Ok(endpoint) = format!("{origin}/customer/{domain}/{local}/did.json").parse() else {
        return state::UNAVAILABLE;
    };
    match get(&endpoint).await {
        Ok(response) => state_for_status(response.status),
        // A refusal still carries the status that IS the answer: the
        // lookup says "nobody registered this" with a 404.
        Err(HttpError::Upstream(failure)) => state_for_status(failure.status),
        Err(error) => {
            log!("email lookup did not complete: {error}");
            state::UNAVAILABLE
        }
    }
}

/// Write the answer to the profile overlay, replacing any earlier one.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn publish(env: &crate::router::CommandEnv, email: &str, state: &'static str) {
    use tonk_common::log;
    use tonk_schema::EmailStatus;

    let Ok(this) = EmailStatus::ENTITY.parse::<dialog_artifacts::Entity>() else {
        return;
    };
    let tonk = env.state().read().await;
    if let Err(error) = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .overlay()
        .assert(EmailStatus::new(this, email.trim().to_owned(), state))
        .write()
        .perform(&tonk.operator)
        .await
    {
        log!("failed to publish the email status: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_splits_an_address_domain_first() {
        assert_eq!(
            split_address("jsmith@example.com"),
            Some(("example.com".to_owned(), "jsmith".to_owned()))
        );
        // The lookup key is normalized, so the form's casing does not
        // decide whether an address is found.
        assert_eq!(
            split_address("  JSmith@Example.COM  "),
            Some(("example.com".to_owned(), "jsmith".to_owned()))
        );
        // `@` splits from the right, so an address whose local part
        // contains one still resolves its domain.
        assert_eq!(
            split_address("a@b@example.com"),
            Some(("example.com".to_owned(), "a@b".to_owned()))
        );
    }

    #[dialog_common::test]
    fn it_refuses_what_it_cannot_turn_into_a_lookup() {
        assert!(split_address("").is_none());
        assert!(split_address("nobody").is_none(), "no domain");
        assert!(split_address("@example.com").is_none(), "no local part");
        assert!(split_address("jsmith@").is_none(), "empty domain");
        // A `/` would split the route into a different path.
        assert!(split_address("a/b@example.com").is_none());
        assert!(split_address(&format!("{}@example.com", "x".repeat(250))).is_none());
    }

    /// The states the form branches on are the statuses the access
    /// service's lookup answers. Pinned here so a change at either end
    /// shows up as a failure rather than as a form that offers the wrong
    /// next step.
    #[dialog_common::test]
    fn it_reads_every_status_the_lookup_answers() {
        assert_eq!(state_for_status(200), state::ACTIVE);
        assert_eq!(state_for_status(202), state::PENDING);
        assert_eq!(state_for_status(404), state::UNREGISTERED);
        assert_eq!(state_for_status(410), state::SUSPENDED);
    }

    /// An address that cannot become a lookup path is answered without
    /// asking anyone.
    #[dialog_common::test]
    fn it_answers_an_unaskable_address_without_a_lookup() {
        assert_eq!(state_for_address("nobody"), Some(state::INVALID));
        assert_eq!(state_for_address("a/b@example.com"), Some(state::INVALID));
        assert_eq!(
            state_for_address("jsmith@example.com"),
            None,
            "a real address has no answer until the service gives one",
        );
    }

    /// Anything else is the service failing to answer. Reading a 429 or
    /// a 502 as "unregistered" would send someone into a creation
    /// ceremony that fails at the end.
    #[dialog_common::test]
    fn it_treats_an_unanswered_lookup_as_unavailable() {
        assert_eq!(state_for_status(429), state::UNAVAILABLE);
        assert_eq!(state_for_status(500), state::UNAVAILABLE);
        assert_eq!(state_for_status(502), state::UNAVAILABLE);
    }
}
