//! Reaching a `tonk` running on this machine.
//!
//! The worker holds the iroh endpoint and the transport; a page holds
//! the `RTCPeerConnection`, because that interface is `[Exposed=Window]`
//! and does not exist here. A page that has dialed the CLI transfers a
//! `MessagePort` in, and [`tonk_rtc::transport::relay::attach`] turns it
//! into a route iroh can use. Everything above that is ordinary dialog:
//! a signed invocation, verified at the far end.
//!
//! # Why this is a route and not a command
//!
//! `commands-not-routes` is right about almost everything a page
//! triggers, and this is one of the exceptions it names: something the
//! worker must answer *before* the page has a branch to subscribe to.
//! "Is a CLI reachable, and which identities does it answer for" is a
//! probe, like `/api/identify` and `/api/site` beside it. There is no
//! durable fact to assert — a peer being up is not something to write to
//! a branch — and nothing would subscribe to it.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;

/// What a page tells the worker when it has a carrier open.
///
/// The port is transferred separately; this is the envelope beside it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Carrier {
    /// The rendezvous phrase the page dialed with, so a worker and a
    /// page that disagree about it fail loudly here rather than as a
    /// route that never matches.
    pub phrase: String,
}

/// What `/api/cli/status` answers with.
///
/// Flat rather than an enum so a page can render it without a match: a
/// CLI that is not there is `reachable: false` with a reason, not an
/// error status, because "no CLI running" is the ordinary case and not
/// a fault.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    /// Whether an invocation completed.
    pub reachable: bool,
    /// The authority the CLI answered for, when it answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// The identity holding it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// The session key that signed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    /// Why not, when it did not answer. Plain prose: a page shows it and
    /// a human reads it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Status {
    /// No CLI answered, and this is why.
    pub fn unreachable(detail: impl Into<String>) -> Self {
        Self {
            reachable: false,
            subject: None,
            profile: None,
            operator: None,
            detail: Some(detail.into()),
        }
    }

    /// A CLI answered, and this is who it said it was.
    pub fn from_greeting(greeting: dialog_effects::peer::Greeting) -> Self {
        Self {
            reachable: true,
            subject: Some(greeting.subject.to_string()),
            profile: Some(greeting.profile.to_string()),
            operator: Some(greeting.operator.to_string()),
            detail: None,
        }
    }
}

/// The endpoint and transport this worker reaches local peers through.
///
/// Built once and kept, because an endpoint's key is its name: rebuilt
/// per request it would be a different peer each time, and every
/// delegation the CLI verified would name a stranger.
pub struct Reach {
    /// The transport pages attach carriers to.
    pub transport: Arc<tonk_rtc::transport::WebRtcTransport>,
    /// The iroh endpoint riding over it.
    pub endpoint: iroh::Endpoint,
}

/// Built on demand and kept for the worker's life.
///
/// The endpoint's key is its name. Rebuilt per request it would be a
/// different peer each time — not a stale route, which iroh re-resolves,
/// but a stranger to anything that had spoken to it.
pub type Lazy = tokio::sync::OnceCell<Arc<Reach>>;

impl Reach {
    /// Bind an endpoint over a fresh transport.
    ///
    /// The key is ephemeral on purpose: this side is the dialer, and
    /// what the CLI verifies is the *invocation's* proof chain, not who
    /// carried it. A persisted key here would be an identity nothing
    /// asks about.
    pub async fn bind() -> Result<Self, String> {
        let transport =
            tonk_rtc::transport::WebRtcTransport::new(tonk_rtc::rendezvous::transport_tag(
                tonk_rtc::rendezvous::RENDEZVOUS,
                tonk_rtc::rendezvous::Side::Dialer,
            ));

        let endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::Empty)
            .crypto_provider(iroh::tls::default_provider())
            .secret_key(iroh::SecretKey::generate())
            .add_custom_transport(transport.clone())
            .bind()
            .await
            .map_err(|error| format!("could not bind an endpoint: {error}"))?;

        Ok(Self {
            transport,
            endpoint,
        })
    }

    /// The address of the `tonk` this rendezvous names.
    ///
    /// The endpoint id is not derivable — it is the CLI's own key — so
    /// it is supplied; the route is, and both ends compute it from the
    /// phrase.
    pub fn peer(
        &self,
        id: iroh::EndpointId,
        phrase: &str,
    ) -> dialog_iroh_remote::site::IrohAddress {
        dialog_iroh_remote::site::IrohAddress::from(iroh::EndpointAddr {
            id,
            addrs: [iroh::TransportAddr::Custom(
                tonk_rtc::transport::rendezvous_addr(phrase, tonk_rtc::rendezvous::Side::Listener),
            )]
            .into_iter()
            .collect(),
        })
    }
}

/// Ask the CLI who it is.
///
/// The whole exchange: a signed `peer::Hello`, carried over iroh, over
/// whatever carrier a page has attached. An unreachable CLI is an
/// answer rather than an error — nothing is running is the ordinary
/// case — so a transport failure becomes `reachable: false` and only a
/// fault in this worker is a status code.
pub async fn status(
    reach: &Reach,
    id: iroh::EndpointId,
    phrase: &str,
    subject: dialog_capability::Did,
    operator: &crate::worker::DefaultOperator,
) -> Status {
    use dialog_capability::{Fork, ForkInvocation, Provider, SiteFork, Subject};
    use dialog_effects::Use;
    use dialog_effects::peer::{Hello, Peer};
    use dialog_iroh_remote::site::{Iroh, IrohFork};

    let site = Iroh::new(dialog_iroh_remote::transport::IrohChannel::new(
        reach.endpoint.clone(),
    ));

    let hello = Subject::from(subject)
        .attenuate(Use)
        .attenuate(Peer)
        .attenuate(Hello);
    let fork: IrohFork<Hello> = Fork::<Iroh, _>::new(hello, reach.peer(id, phrase)).into();

    let invocation = match fork.authorize(operator).await {
        Ok(invocation) => invocation,
        // Failing to sign is this side's problem, and saying "the CLI is
        // unreachable" would send someone looking in the wrong place.
        Err(error) => return Status::unreachable(format!("could not sign the request: {error}")),
    };

    match Provider::<ForkInvocation<Iroh, Hello>>::execute(&site, invocation).await {
        Ok(greeting) => Status::from_greeting(greeting),
        Err(error) => Status::unreachable(error.to_string()),
    }
}

/// Take a carrier a page has opened and make it a route.
///
/// The page owns the `RTCPeerConnection` and relays its datagrams over
/// the transferred port; this end knows nothing about WebRTC and only
/// hands the port to the transport. Both sides name the peer from the
/// rendezvous phrase, so no address is exchanged here either.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn handle_carrier(
    state: crate::router::AppState,
    client: crate::router::ClientId,
    ports: js_sys::Array,
) {
    use tonk_common::log;
    use wasm_bindgen::JsCast;

    let Some(port) = ports.get(0).dyn_into::<web_sys::MessagePort>().ok() else {
        log!("cli: carrier from {client:?} had no transferred port; dropping");
        return;
    };

    let reach = {
        let tonk = state.read().await;
        tonk.reach.clone()
    };
    let reach = match reach
        .get_or_try_init(|| async { Reach::bind().await.map(Arc::new) })
        .await
    {
        Ok(reach) => reach.clone(),
        Err(error) => {
            log!("cli: no endpoint to attach a carrier to: {error}");
            return;
        }
    };

    // The page is the dialer, so that is the name its datagrams arrive
    // under — and the name replies must be addressed to.
    let peer = tonk_rtc::transport::rendezvous_addr(
        tonk_rtc::rendezvous::RENDEZVOUS,
        tonk_rtc::rendezvous::Side::Dialer,
    );
    tonk_rtc::transport::relay::attach(&reach.transport, peer, port);
    log!("cli: a carrier from {client:?} is now a route");
}

/// `GET /api/cli/status?peer=<did:key>` — ask a local `tonk` who it is.
///
/// `peer` is the CLI's `did:key`, printed by `tonk rtc serve`. It is not
/// derivable: the rendezvous phrase names the *route*, and the endpoint
/// key is the CLI's own identity.
#[axum_wasm_macros::wasm_compat]
pub async fn status_route(
    axum::extract::State(state): axum::extract::State<crate::router::AppState>,
    axum::extract::Query(query): axum::extract::Query<StatusQuery>,
) -> Result<axum::Json<Status>, crate::TonkWorkerError> {
    let peer = match query.peer.parse::<dialog_iroh_remote::site::IrohAddress>() {
        Ok(peer) => peer,
        Err(error) => {
            return Ok(axum::Json(Status::unreachable(format!(
                "that is not a peer address: {error}"
            ))));
        }
    };

    let (reach, operator, subject) = {
        let tonk = state.read().await;
        (
            tonk.reach.clone(),
            tonk.operator.clone(),
            tonk.profile.did(),
        )
    };

    // No carrier attached yet means no page has dialed, which is the
    // ordinary state before anyone tries — not a fault.
    let Some(reach) = reach.get().cloned() else {
        return Ok(axum::Json(Status::unreachable(
            "no page has opened a carrier to a local tonk yet",
        )));
    };

    Ok(axum::Json(
        status(
            &reach,
            *peer.endpoint(),
            tonk_rtc::rendezvous::RENDEZVOUS,
            subject,
            &operator,
        )
        .await,
    ))
}

/// The `peer` a status probe is about.
#[derive(Debug, Clone, Deserialize)]
pub struct StatusQuery {
    /// The CLI's `did:key`, as `tonk rtc serve` prints it.
    pub peer: String,
}
