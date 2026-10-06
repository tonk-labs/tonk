//! Run `account/activate`: present an activation link's invocation to the
//! access service, and say how it went.
//!
//! The link in an activation email carries a complete, service-signed
//! `/customer/activate` invocation. The page that opens it asserts the
//! command with that invocation and an entity of its own; this handler
//! posts the bytes to the deployment's `/ucan/` endpoint and records the
//! outcome on that entity, in the profile's session overlay, where the
//! page reads it. Nothing else is recorded: the account's remote is
//! already attached, and the next sync finds the gate open.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use dialog_artifacts::Entity;
use dialog_query::{Cardinality, the};
use tonk_common::log;
use url::Url;

use super::http::{HttpError, post_cbor};
use crate::router::AppState;

/// How an activation went, as the page reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The service activated the account.
    Activated,
    /// The link cannot activate anything, now or later.
    Refused(String),
    /// The attempt failed in a way another press may not.
    Failed(String),
}

impl Outcome {
    fn status(&self) -> &'static str {
        match self {
            Self::Activated => "activated",
            Self::Refused(_) => "refused",
            Self::Failed(_) => "failed",
        }
    }

    fn detail(&self) -> &str {
        match self {
            Self::Activated => "",
            Self::Refused(detail) | Self::Failed(detail) => detail,
        }
    }
}

/// The bytes a link's `ucan` parameter carries.
pub(crate) fn decode(invocation: &str) -> Result<Vec<u8>, Outcome> {
    if invocation.is_empty() {
        return Err(Outcome::Refused(
            "This activation link is incomplete. Open the exact link from your email.".into(),
        ));
    }
    URL_SAFE_NO_PAD.decode(invocation).map_err(|_| {
        Outcome::Refused(
            "This activation link is damaged. Open the exact link from your email.".into(),
        )
    })
}

/// What the service's answer to the invocation means for the page.
pub(crate) fn outcome(answer: Result<(), HttpError>) -> Outcome {
    match answer {
        Ok(()) => Outcome::Activated,
        Err(HttpError::Upstream(failure)) if failure.code.as_deref() == Some("Unauthorized") => {
            Outcome::Refused(
                "This activation link has expired. Sign in on your device to get a fresh one."
                    .into(),
            )
        }
        Err(HttpError::Upstream(failure)) if failure.status >= 500 => {
            Outcome::Failed("The service could not activate the account. Try again.".into())
        }
        // What the service said is logged, not shown: it is written for
        // whoever runs the service, not for the person holding the link.
        Err(HttpError::Upstream(_)) => Outcome::Refused(
            "This activation link did not work. Sign in on your device to get a fresh one.".into(),
        ),
        Err(HttpError::Timeout | HttpError::Transport(_)) => {
            Outcome::Failed("The service could not be reached. Try again.".into())
        }
    }
}

async fn activate(invocation: &str) -> Outcome {
    let bytes = match decode(invocation) {
        Ok(bytes) => bytes,
        Err(refused) => return refused,
    };
    let endpoint = super::repository::app_origin()
        .and_then(|origin| Url::parse(&format!("{}/ucan/", origin.trim_end_matches('/'))).ok());
    let Some(endpoint) = endpoint else {
        return Outcome::Failed("The service could not be reached. Try again.".into());
    };
    let answer = post_cbor(&endpoint, &bytes).await.map(|_| ());
    if let Err(error) = &answer {
        log!("account activation: {error}");
    }
    outcome(answer)
}

/// Record how the activation `receipt` names went, on `branch`: the one the
/// command was asserted on, which is the one the page that asked is
/// watching.
async fn report(state: &AppState, branch: &str, receipt: &Entity, outcome: &Outcome) {
    let tonk = state.read().await;
    if let Err(error) = tonk
        .reactor
        .profile_repository()
        .branch(branch)
        .overlay()
        .assert(
            the!("xyz.tonk.account-activation/status")
                .of(receipt.clone())
                .is(outcome.status().to_owned())
                .cardinality(Cardinality::One),
        )
        .assert(
            the!("xyz.tonk.account-activation/detail")
                .of(receipt.clone())
                .is(outcome.detail().to_owned())
                .cardinality(Cardinality::One),
        )
        .write()
        .perform(&tonk.operator)
        .await
    {
        log!("account activation: failed to publish the outcome: {error}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::ActivateAccount>
    for crate::router::CommandEnv
{
    async fn execute(&self, command: tonk_schema::command::ActivateAccount) {
        let outcome = activate(&command.invocation.0).await;
        report(self.state(), &self.origin().branch, &command.this, &outcome).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::http::UpstreamFailure;

    fn upstream(status: u16, code: Option<&str>, message: &str) -> Result<(), HttpError> {
        Err(HttpError::Upstream(UpstreamFailure {
            status,
            code: code.map(str::to_owned),
            message: message.to_owned(),
        }))
    }

    #[dialog_common::test]
    fn it_decodes_the_bytes_a_link_carries() {
        assert_eq!(decode("aGVsbG8"), Ok(b"hello".to_vec()));
    }

    #[dialog_common::test]
    fn it_refuses_a_link_with_no_invocation() {
        assert!(matches!(decode(""), Err(Outcome::Refused(said)) if said.contains("incomplete")));
    }

    #[dialog_common::test]
    fn it_refuses_a_link_that_is_not_base64url() {
        assert!(
            matches!(decode("not base64!"), Err(Outcome::Refused(said)) if said.contains("damaged"))
        );
    }

    #[dialog_common::test]
    fn it_reads_the_services_answers() {
        assert_eq!(outcome(Ok(())), Outcome::Activated);
        assert!(matches!(
            outcome(upstream(401, Some("Unauthorized"), "no")),
            Outcome::Refused(said) if said.contains("expired")
        ));
        assert!(matches!(
            outcome(upstream(503, None, "down")),
            Outcome::Failed(_)
        ));
        assert!(matches!(
            outcome(upstream(400, Some("Invalid"), "decode: byte 104")),
            Outcome::Refused(said) if said.contains("did not work") && !said.contains("104")
        ));
        assert!(matches!(
            outcome(Err(HttpError::Timeout)),
            Outcome::Failed(_)
        ));
    }
}
