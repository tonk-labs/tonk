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
/// The vocabulary itself lives with the concept in `tonk-schema`, so
/// the worker that writes these strings and the registration form that
/// routes on them cannot drift apart.
pub(crate) use tonk_schema::email_state as state;

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
    /// Decodes the current shape, and the deprecated one a
    /// branch seeded before the migration still asserts.
    command: crate::reactor::Migrated<
        tonk_schema::command::CheckEmail,
        tonk_schema::command::legacy::CheckEmail,
    >,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl CheckEmailHandler {
    pub(crate) fn new() -> Self {
        Self {
            command: crate::reactor::Migrated::new(),
        }
    }

    /// The address to look up, or `None` when these facts are not a
    /// lookup (or carry a blank address).
    fn email(&self, facts: &crate::reactor::EntityFacts) -> Option<String> {
        self.command
            .decode(facts)
            .map(|command| command.email.0)
            .filter(|email| !email.trim().is_empty())
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl crate::reactor::CommandHandler<crate::router::CommandEnv> for CheckEmailHandler {
    fn trigger_attributes(&self) -> &[String] {
        self.command.trigger_attributes()
    }

    fn matches(&self, facts: &crate::reactor::EntityFacts) -> bool {
        self.email(facts).is_some()
    }

    fn run(
        &self,
        facts: &crate::reactor::EntityFacts,
        env: &crate::router::CommandEnv,
    ) -> crate::reactor::RunFuture {
        let email = self.email(facts);
        let env = env.clone();

        Box::pin(async move {
            let Some(email) = email else {
                return;
            };
            // Say the lookup is in flight BEFORE making it. The form
            // renders the row and nothing else, so without this the
            // wait would have to be painted into the DOM by the form
            // itself, leaving two sources of truth that disagree while
            // the lookup runs.
            publish(&env, &email, state::CHECKING).await;
            let (state, service) = lookup(&email).await;
            publish(&env, &email, state).await;
            // The document says where the account syncs as well as who
            // it is, so one lookup answers both. Held for the login
            // that follows: a device with only an address has nowhere
            // else to learn the service, and the origin is a guess that
            // is right only when both devices are on one deployment.
            if let Some(service) = service {
                remember_service(&service);
            }
        })
    }
}

/// Ask the access service about `email`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn lookup(email: &str) -> (&'static str, Option<String>) {
    use super::http::{HttpError, get};
    use tonk_common::log;

    if let Some(state) = state_for_address(email) {
        return (state, None);
    }
    let Some((domain, local)) = split_address(email) else {
        return (state::INVALID, None);
    };
    let Some(origin) = super::repository::worker_origin() else {
        return (state::UNAVAILABLE, None);
    };
    let Ok(endpoint) = format!("{origin}/customer/{domain}/{local}/did.json").parse() else {
        return (state::UNAVAILABLE, None);
    };
    match get(&endpoint).await {
        Ok(response) => (
            state_for_status(response.status),
            service_endpoint(&response.body),
        ),
        // A refusal still carries the status that IS the answer: the
        // lookup says "nobody registered this" with a 404.
        Err(HttpError::Upstream(failure)) => (state_for_status(failure.status), None),
        Err(error) => {
            log!("email lookup did not complete: {error}");
            (state::UNAVAILABLE, None)
        }
    }
}

/// The sync address a resolved DID document names, if it names one.
///
/// A document from a service that predates the `service` block simply
/// has none, and the caller keeps whatever it already knew.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn service_endpoint(body: &[u8]) -> Option<String> {
    let document: serde_json::Value = serde_json::from_slice(body).ok()?;
    document
        .get("service")?
        .as_array()?
        .iter()
        .find(|entry| entry.get("type").and_then(|kind| kind.as_str()) == Some("TonkAccessService"))
        .and_then(|entry| entry.get("serviceEndpoint"))
        .and_then(|endpoint| endpoint.as_str())
        .map(ToString::to_string)
}

/// Write the answer to the profile overlay, replacing any earlier one.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn publish(env: &crate::router::CommandEnv, email: &str, state: &'static str) {
    let tonk = env.state().read().await;
    record(&tonk, email, state).await;
}

/// The lookup vocabulary for a registration status.
///
/// The form reads one set of words whether they came from the lookup or
/// from the service's own receipt, so a registration answers in the
/// lookup's terms rather than in its own.
pub(crate) fn state_for_customer(status: tonk_account::customer::CustomerStatus) -> &'static str {
    use tonk_account::customer::CustomerStatus;
    match status {
        CustomerStatus::Registered => state::PENDING,
        CustomerStatus::Active => state::ACTIVE,
        CustomerStatus::Suspended => state::SUSPENDED,
    }
}

/// Record an answer about `email` on the profile overlay.
///
/// Shared by the lookup and by [`record_customer_status`], which is the
/// other place an answer about an address is learned: activation is a
/// NEW answer about it, and without writing one here an address checked
/// before registering stayed `unregistered` in the overlay forever —
/// so the form kept offering to create an account for one that had just
/// finished activating.
///
/// [`record_customer_status`]: crate::router::customer::record_customer_status
pub(crate) async fn record(tonk: &crate::worker::TonkState, email: &str, answer: &'static str) {
    use tonk_common::log;
    use tonk_schema::EmailStatus;

    let email = email.trim();
    if email.is_empty() {
        return;
    }
    let Ok(this) = EmailStatus::ENTITY.parse::<dialog_artifacts::Entity>() else {
        return;
    };
    if let Err(error) = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .overlay()
        .assert(EmailStatus::new(this, email.to_owned(), answer))
        .write()
        .perform(&tonk.operator)
        .await
    {
        log!("failed to publish the email status: {error}");
    }
}

/// Runs `account/register`: raises the signup ceremony in the page.
///
/// The worker cannot create an account. WebAuthn needs a `window` and a
/// user gesture, and a service worker has neither, so this asks the
/// originating client to authorize with a passkey and stops there.
///
/// Nothing is awaited. The ceremony's outcome reaches every reader as
/// facts — `AccountCustomer` appears at enrollment and gains a provider
/// at activation — and the form is already subscribed to them. A handler
/// that blocked on the ceremony would be holding a command open across a
/// dialog the user might never finish.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) struct RegisterAccountHandler {
    /// Decodes the current shape, and the deprecated one a
    /// branch seeded before the migration still asserts.
    command: crate::reactor::Migrated<
        tonk_schema::command::RegisterAccount,
        tonk_schema::command::legacy::RegisterAccount,
    >,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl RegisterAccountHandler {
    pub(crate) fn new() -> Self {
        Self {
            command: crate::reactor::Migrated::new(),
        }
    }

    /// The address to register, or `None` when these facts are not a
    /// registration (or carry an unparseable address).
    fn email(&self, facts: &crate::reactor::EntityFacts) -> Option<String> {
        self.command
            .decode(facts)
            .map(|command| command.email.0)
            .filter(|email| split_address(email).is_some())
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl crate::reactor::CommandHandler<crate::router::CommandEnv> for RegisterAccountHandler {
    fn trigger_attributes(&self) -> &[String] {
        self.command.trigger_attributes()
    }

    fn matches(&self, facts: &crate::reactor::EntityFacts) -> bool {
        self.email(facts).is_some()
    }

    fn run(
        &self,
        facts: &crate::reactor::EntityFacts,
        env: &crate::router::CommandEnv,
    ) -> crate::reactor::RunFuture {
        use tonk_common::log;

        let email = self.email(facts);
        let env = env.clone();

        Box::pin(async move {
            let Some(email) = email else {
                return;
            };
            let Some(client) = env.client() else {
                log!("account/register: no page asked for this, so no ceremony can run");
                return;
            };
            // The address rides on the overlay rather than in the
            // request: `WebAuthnRequest` carries a discriminator and
            // nothing else, and the page reads what it needs from the
            // row it is already watching.
            publish(&env, &email, state::PENDING_CEREMONY).await;
            if let Err(error) = super::navigate::request_webauthn(
                client,
                tonk_worker_api::WebAuthnKind::CreateAccount,
            )
            .await
            {
                log!("account/register: the page could not be asked: {error}");
                publish(&env, &email, state::UNAVAILABLE).await;
            }
        })
    }
}

#[cfg(test)]
mod tests {

    /// Activation is a new answer about the address.
    ///
    /// `EmailStatus` used to be written only by the lookup handler, so
    /// an address checked before registering stayed `unregistered`
    /// forever — and the form kept offering to create an account for one
    /// that had just finished activating. The states below are what
    /// `republish` maps a registration status onto, in the lookup's own
    /// vocabulary so the form reads one set of words.
    #[dialog_common::test]
    fn it_answers_with_the_lookup_vocabulary_after_activation() {
        // `Active` is what an opened activation link produces, and the
        // form must route it to sign-in rather than creation.
        assert_eq!(state_for_status(200), state::ACTIVE);
        // Enrolled but unconfirmed.
        assert_eq!(state_for_status(202), state::PENDING);
        assert_eq!(state_for_status(410), state::SUSPENDED);
    }

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

// The sync address the last lookup resolved, for the login that follows
// it. Thread-local rather than a fact: it is learned before an account
// exists to hang it on, and it is consumed within the same session by
// the sign-in the lookup was run for.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
thread_local! {
    static RESOLVED_SERVICE: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// Keep the address a lookup resolved.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn remember_service(endpoint: &str) {
    RESOLVED_SERVICE.with(|cell| *cell.borrow_mut() = Some(endpoint.to_owned()));
}

/// The address the last lookup resolved, if one did.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn resolved_service() -> Option<String> {
    RESOLVED_SERVICE.with(|cell| cell.borrow().clone())
}
