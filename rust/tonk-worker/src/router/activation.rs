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
use tonk_analytics::account::{
    AccountAction, AccountOutcome, FailureKind, HttpStatusClass, ServiceCode, Stage, Trigger,
};
use tonk_common::log;
use url::Url;

use super::account_journey::Attempt;
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

/// How an attempt that never reached the service ended, for the account
/// journey: the link carried nothing to present.
fn unpresented() -> (Stage, AccountOutcome) {
    (
        Stage::Input,
        AccountOutcome::terminal_failure(FailureKind::InvalidInput),
    )
}

/// How the attempt ended, for the account journey, from the service's
/// answer: activated, turned away for good, or failed in a way worth
/// another try, with the class of status and the code it answered.
pub(crate) fn journey_end(answer: &Result<(), HttpError>) -> (Stage, AccountOutcome) {
    let outcome = match answer {
        Ok(()) => return (Stage::Complete, AccountOutcome::success()),
        Err(HttpError::Upstream(failure)) if failure.code.as_deref() == Some("Unauthorized") => {
            AccountOutcome::terminal_failure(FailureKind::AccessDenied)
                .with_http_status_class(HttpStatusClass::ClientError)
                .with_service_code(ServiceCode::Unauthorized)
        }
        Err(HttpError::Upstream(failure)) if failure.status >= 500 => {
            AccountOutcome::retryable(FailureKind::ServiceUnavailable)
                .with_http_status_class(HttpStatusClass::ServerError)
        }
        Err(HttpError::Upstream(_)) => AccountOutcome::terminal_failure(FailureKind::AccessDenied)
            .with_http_status_class(HttpStatusClass::ClientError),
        Err(HttpError::Timeout | HttpError::Transport(_)) => {
            AccountOutcome::retryable(FailureKind::Network)
        }
    };
    (Stage::AccessService, outcome)
}

/// Present the link's invocation, for the attempt `attempt` tells of.
async fn activate(invocation: &str, mut attempt: Attempt) -> Outcome {
    let bytes = match decode(invocation) {
        Ok(bytes) => bytes,
        Err(refused) => {
            let (stage, ended) = unpresented();
            attempt.end(stage, ended);
            return refused;
        }
    };
    attempt.reached(Stage::AccessService);
    let endpoint = super::repository::app_origin()
        .and_then(|origin| Url::parse(&format!("{}/ucan/", origin.trim_end_matches('/'))).ok());
    let Some(endpoint) = endpoint else {
        attempt.end(
            Stage::AccessService,
            AccountOutcome::retryable(FailureKind::Network),
        );
        return Outcome::Failed("The service could not be reached. Try again.".into());
    };
    let answer = post_cbor(&endpoint, &bytes).await.map(|_| ());
    if let Err(error) = &answer {
        log!("account activation: {error}");
    }
    let (stage, ended) = journey_end(&answer);
    attempt.end(stage, ended);
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
        let attempt = Attempt::begin(self.client(), AccountAction::ActivateAccount, Trigger::User);
        let outcome = activate(&command.invocation.0, attempt).await;
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

    /// The outcomes the activation page told the account journey, kept
    /// now that the worker presents the link.
    #[dialog_common::test]
    fn it_tells_the_journey_how_presenting_the_link_ended() {
        assert_eq!(
            journey_end(&Ok(())),
            (Stage::Complete, AccountOutcome::success())
        );
        assert_eq!(
            journey_end(&upstream(401, Some("Unauthorized"), "expired")),
            (
                Stage::AccessService,
                AccountOutcome::terminal_failure(FailureKind::AccessDenied)
                    .with_http_status_class(HttpStatusClass::ClientError)
                    .with_service_code(ServiceCode::Unauthorized)
            )
        );
        assert_eq!(
            journey_end(&upstream(503, None, "down")),
            (
                Stage::AccessService,
                AccountOutcome::retryable(FailureKind::ServiceUnavailable)
                    .with_http_status_class(HttpStatusClass::ServerError)
            )
        );
        assert_eq!(
            journey_end(&upstream(400, Some("Invalid"), "no")),
            (
                Stage::AccessService,
                AccountOutcome::terminal_failure(FailureKind::AccessDenied)
                    .with_http_status_class(HttpStatusClass::ClientError)
            )
        );
        assert_eq!(
            journey_end(&Err(HttpError::Timeout)),
            (
                Stage::AccessService,
                AccountOutcome::retryable(FailureKind::Network)
            )
        );
        assert_eq!(
            unpresented(),
            (
                Stage::Input,
                AccountOutcome::terminal_failure(FailureKind::InvalidInput)
            )
        );
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

/// The command as a page asserts it, through the worker's own routes: the
/// handler runs, and the page reads the outcome off the entity it named.
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod command_tests {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    use ::axum::Router;
    use ::axum::body::Body;
    use ::axum::http::{Request, StatusCode};
    use tower::ServiceExt as _;

    async fn post(app: &Router, operation: &str, body: serde_json::Value) -> serde_json::Value {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/repository/profile:tonk/branch/main/{operation}"
                    ))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{operation}");
        let bytes = ::axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    }

    /// What the page reads back: the outcome recorded on `receipt`.
    async fn outcome(app: &Router, receipt: &str) -> Option<(String, String)> {
        let field = |name: &str| {
            serde_json::json!({
                "the": format!("xyz.tonk.account-activation/{name}"),
                "as": "Text",
                "cardinality": "one"
            })
        };
        let rows = post(
            app,
            "query",
            serde_json::json!({
                "predicate": { "with": { "status": field("status"), "detail": field("detail") } },
                "terms": {
                    "this": receipt,
                    "status": { "?": { "name": "status" } },
                    "detail": { "?": { "name": "detail" } }
                }
            }),
        )
        .await;
        let fields = rows.as_array()?.first()?.get("fields")?.clone();
        Some((
            fields["status"].as_str()?.to_owned(),
            fields["detail"].as_str()?.to_owned(),
        ))
    }

    async fn activate(app: &Router, receipt: &str, invocation: &str) {
        post(
            app,
            "transact",
            serde_json::json!({ "claims": [{ "op": "assert", "application": {
                "predicate": { "kind": "transient", "concept": { "with": {
                    "invocation": {
                        "the": "xyz.tonk.command.activate-account/invocation",
                        "as": "Text"
                    }
                } } },
                "parameters": { "this": receipt, "invocation": invocation }
            } }] }),
        )
        .await;
    }

    /// A link that cannot be an invocation is refused before anything is
    /// sent, and the page that asked is told on the entity it named.
    #[dialog_common::test]
    async fn it_records_a_refusal_where_the_page_that_asked_reads_it() {
        let (app, _state, _lsp) =
            crate::router::api_router_with_state(crate::router::tests::test_state().await);
        let receipt = "urn:uuid:11111111-1111-4111-8111-111111111111";

        activate(&app, receipt, "not base64url!").await;

        // The handler runs after the transact answers.
        let mut recorded = None;
        for _ in 0..100 {
            recorded = outcome(&app, receipt).await;
            if recorded.is_some() {
                break;
            }
            crate::router::tests::wasm_yield().await;
        }
        let (status, detail) = recorded.expect("the outcome is recorded");
        assert_eq!(status, "refused");
        assert!(detail.contains("damaged"), "got {detail:?}");
    }

    /// Each press names its own entity, so one press's outcome is never
    /// read as another's.
    #[dialog_common::test]
    async fn it_keeps_each_requests_outcome_apart() {
        let (app, _state, _lsp) =
            crate::router::api_router_with_state(crate::router::tests::test_state().await);
        let first = "urn:uuid:22222222-2222-4222-8222-222222222222";
        let second = "urn:uuid:33333333-3333-4333-8333-333333333333";

        activate(&app, first, "%%%").await;
        for _ in 0..100 {
            if outcome(&app, first).await.is_some() {
                break;
            }
            crate::router::tests::wasm_yield().await;
        }

        assert!(outcome(&app, first).await.is_some());
        assert_eq!(outcome(&app, second).await, None);
    }
}
