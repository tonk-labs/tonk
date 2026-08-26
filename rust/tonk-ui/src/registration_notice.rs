//! The account panel's registration row, kept live by a subscription.
//!
//! Registration state is a fact on profile main (`xyz.tonk.account`),
//! written wherever the service's answer is learned. Reading it through
//! a subscription rather than a fetch is what lets this row notice an
//! activation it did not perform: the emailed link is routinely opened
//! somewhere else -- a phone mail client, another browser -- and the
//! device that opened it, or the sync that stopped being refused,
//! records the fact. It reaches every device on the account from there,
//! and every tab showing this row repaints.
//!
//! The panel previously fetched `/api/customer` once per dashboard
//! render and read untyped JSON, so an activation elsewhere left this
//! tab saying "waiting for email confirmation" until someone reloaded.

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use js_sys::{JSON, Reflect};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tonk_host::consumer::{self, Subscription};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use wasm_bindgen::JsValue;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use web_sys::{Element, HtmlElement};

/// Identifies this subscription's frames among any others the element
/// carries.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const SUB_TAG: &str = "account-registration";

/// The branch registration facts live on, which is the profile's own
/// main branch rather than any space.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const PROFILE_MAIN: &str = "main";

/// How far the account got, as this row needs to render it.
///
/// The worker's `router::customer::Registration` is the same reading and
/// is `pub(crate)` there, so the discrimination is restated rather than
/// shared. The rule it encodes is the worker's: a recorded provider
/// means registration finished, because the access service names one
/// only once it actually serves the customer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registration {
    /// Nothing recorded a registration for this account.
    Unregistered,
    /// Enrolled, but the emailed link has not been opened, so nothing
    /// serves this account yet.
    AwaitingActivation {
        /// Where the link was sent, for the notice to name.
        email: String,
    },
    /// Registered and served.
    Served,
    /// Registered, then withdrawn. No email confirms this away.
    Suspended,
}

impl Registration {
    /// Read a registration from the fact's fields.
    ///
    /// Mirrors the worker's `registration()`, including the arm that
    /// keeps an `Active` account with no recorded address out of
    /// `AwaitingActivation`: the status write can land before the one
    /// carrying the provider, and reading that as unconfirmed would tell
    /// a user to go open an email they already opened.
    #[cfg_attr(
        not(all(target_arch = "wasm32", target_os = "unknown")),
        allow(dead_code)
    )]
    fn read(status: &str, email: &str, provider: &str) -> Self {
        if status == "Suspended" {
            return Self::Suspended;
        }
        if !provider.is_empty() || status == "Active" {
            return Self::Served;
        }
        if email.is_empty() {
            return Self::Unregistered;
        }
        Self::AwaitingActivation {
            email: email.to_owned(),
        }
    }

    /// The label for the registration row.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Served => "Active",
            Self::AwaitingActivation { .. } => "Waiting for email confirmation",
            Self::Suspended => "Suspended",
            Self::Unregistered => "Not registered",
        }
    }

    /// The pending-activation banner, when one belongs on screen.
    pub fn notice(&self) -> Option<String> {
        match self {
            Self::AwaitingActivation { email } if !email.is_empty() => Some(format!(
                "Sync activation pending: open the link we emailed to {email}."
            )),
            Self::AwaitingActivation { .. } => {
                Some("Sync activation pending: open the link in your activation email.".to_string())
            }
            _ => None,
        }
    }
}

/// Subscribe `host` to `account`'s registration fact.
///
/// Frames arrive as `consumer.reset(..)` / `.update(..)` on the element,
/// which the component routes back into [`apply_frame`]. The returned
/// [`Subscription`] cancels upstream when dropped, so the caller holds
/// it for as long as the row is on screen.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub fn subscribe(host: &HtmlElement, account: &str) -> Result<Subscription, String> {
    let consumer: Element = host.clone().into();
    let body = query_body(account)?;
    let tag = JsValue::from_str(SUB_TAG);
    consumer::subscribe_with_route(&consumer, &body, Some(&tag), None, Some(PROFILE_MAIN), true)
        .map_err(|error| format!("registration subscribe failed: {error:?}"))
}

/// The subscribe body for one account's registration fact.
///
/// Built as JSON directly, the way `<tonk-sync-status>` and
/// `<tonk-site>` build theirs: the concept is fixed and known, so a
/// typed-query-to-wire conversion would buy nothing here.
///
/// `this` is pinned to the account entity, which is its DID, and every
/// field is read back as a variable. `provider` is optional on the
/// concept, so the row still resolves for an account that enrolled and
/// has not confirmed its address -- which is exactly the account this
/// notice exists to render.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn query_body(account: &str) -> Result<JsValue, String> {
    let body = format!(
        r#"{{
      "predicate": {{ "with": {{
        "status": {{ "the": "xyz.tonk.account/customer-status", "as": "String", "cardinality": "one" }},
        "email": {{ "the": "xyz.tonk.account/customer-email", "as": "String", "cardinality": "one" }},
        "provider": {{ "the": "xyz.tonk.account/provider-address", "as": "String", "cardinality": "one" }}
      }} }},
      "terms": {{
        "this": "{account}",
        "status": {{ "?": {{ "name": "status" }} }},
        "email": {{ "?": {{ "name": "email" }} }},
        "provider": {{ "?": {{ "name": "provider" }} }}
      }}
    }}"#
    );
    JSON::parse(&body).map_err(|error| format!("registration query JSON parse: {error:?}"))
}

/// Read a registration out of a `reset` frame.
///
/// The frame is the conclusion list. An empty one means no fact has been
/// recorded, which is [`Registration::Unregistered`] -- unlike the sync
/// disc, an empty frame here is information rather than a gap, because
/// an account that never enrolled genuinely has no row.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub fn read_frame(payload: &JsValue) -> Registration {
    let conclusions = js_sys::Array::from(payload);
    let first = conclusions.get(0);
    if first.is_undefined() || first.is_null() {
        return Registration::Unregistered;
    }
    read_conclusion(&first)
}

/// Read a registration out of an `update` frame's `asserted` rows.
///
/// `None` when the delta carried no assertion. Every field is
/// cardinality-one, so a change supersedes the prior value and the last
/// asserted row is the current one. A bare retraction leaves the row
/// alone: registration facts are superseded, not withdrawn, so a retract
/// with nothing after it is a moment mid-write rather than a state.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub fn read_delta(payload: &JsValue) -> Option<Registration> {
    let asserted = Reflect::get(payload, &"asserted".into()).ok()?;
    let rows = js_sys::Array::from(&asserted);
    let last = rows.get(rows.length().checked_sub(1)?);
    (!last.is_undefined() && !last.is_null()).then(|| read_conclusion(&last))
}

/// Read one conclusion's fields.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn read_conclusion(conclusion: &JsValue) -> Registration {
    // An optional field arrives as `null` when unset, which `as_string`
    // already answers `None` for, so absent and empty converge here.
    let field = |name: &str| -> String {
        Reflect::get(conclusion, &"fields".into())
            .ok()
            .and_then(|fields| Reflect::get(&fields, &name.into()).ok())
            .and_then(|value| value.as_string())
            .unwrap_or_default()
    };
    Registration::read(&field("status"), &field("email"), &field("provider"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_reads_a_served_account() {
        assert_eq!(
            Registration::read("Active", "a@example.com", "https://tonk.network"),
            Registration::Served
        );
    }

    #[dialog_common::test]
    fn it_reads_an_account_waiting_on_its_email() {
        assert_eq!(
            Registration::read("Registered", "a@example.com", ""),
            Registration::AwaitingActivation {
                email: "a@example.com".to_string()
            }
        );
    }

    /// The status write can land before the one carrying the provider.
    /// Reading that as unconfirmed would tell a user to open an email
    /// they already opened, so `Active` wins over an absent address.
    #[dialog_common::test]
    fn it_does_not_call_an_active_account_unconfirmed() {
        assert_eq!(
            Registration::read("Active", "a@example.com", ""),
            Registration::Served
        );
    }

    /// Suspension outranks a recorded provider: the address is still
    /// there, and it is no longer served.
    #[dialog_common::test]
    fn it_lets_suspension_outrank_a_recorded_provider() {
        assert_eq!(
            Registration::read("Suspended", "a@example.com", "https://tonk.network"),
            Registration::Suspended
        );
    }

    #[dialog_common::test]
    fn it_reads_a_blank_fact_as_unregistered() {
        assert_eq!(Registration::read("", "", ""), Registration::Unregistered);
    }

    /// A status this build does not recognise must not read as served.
    /// Only `Active` and a recorded provider mean served, so anything
    /// newer falls through to the unregistered end rather than claiming
    /// an account works.
    #[dialog_common::test]
    fn it_fails_closed_on_an_unrecognised_status() {
        assert_eq!(
            Registration::read("Deprovisioning", "a@example.com", ""),
            Registration::AwaitingActivation {
                email: "a@example.com".to_string()
            }
        );
        assert_eq!(
            Registration::read("Deprovisioning", "", ""),
            Registration::Unregistered
        );
    }

    #[dialog_common::test]
    fn it_names_the_address_in_the_pending_notice() {
        let pending = Registration::AwaitingActivation {
            email: "a@example.com".to_string(),
        };
        assert!(
            pending
                .notice()
                .expect("a pending registration nags")
                .contains("a@example.com")
        );
        assert_eq!(pending.label(), "Waiting for email confirmation");
    }

    /// Only a pending activation nags. A served account showing a
    /// "check your email" banner is the bug this row exists to fix.
    #[dialog_common::test]
    fn it_nags_only_while_activation_is_pending() {
        assert_eq!(Registration::Served.notice(), None);
        assert_eq!(Registration::Suspended.notice(), None);
        assert_eq!(Registration::Unregistered.notice(), None);
    }
}
