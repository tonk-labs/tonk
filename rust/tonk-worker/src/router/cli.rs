//! Reaching a `tonk` running on this machine.
//!
//! The worker holds the iroh endpoint and the transport; a page holds
//! the `RTCPeerConnection`, because that interface is `[Exposed=Window]`
//! and does not exist here. A page that has dialed the CLI transfers a
//! `MessagePort` in, and [`tonk_rtc::transport::relay::attach`] turns it
//! into a route iroh can use. Everything above that is ordinary dialog:
//! a signed invocation, verified at the far end.
//!
//! # Why this is a command and not a route
//!
//! This began as two routes, on the argument that a reachability probe
//! is something the worker must answer before the page has a branch to
//! subscribe to. That was wrong twice over. The page *does* have a
//! branch — the profile one it is already rendering — and "is a CLI
//! reachable" is exactly a fact to put on it, just not a durable one:
//! an overlay stamp, live for the session, keyed on the `state:cli`
//! singleton the way `tonk:sync` keys `state:here`.
//!
//! Asking is a user action, so it is a command. The difference is not
//! stylistic: a route answers one caller once, while the command's
//! outcome reaches every tab showing the network page through the
//! subscription it already holds, and the same ask runs from a test or
//! the CLI with no second code path.

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

/// What asking a peer who it is answers with.
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

/// What asking a peer what it holds answers with.
///
/// Flat and always-200 for the same reason [`Status`] is: a CLI that is
/// not there is `reachable: false` with a reason, because nothing
/// running is the ordinary case and not a fault. An empty `spaces` on a
/// reachable CLI is its own answer — that peer holds none.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inventory {
    /// Whether the CLI answered.
    pub reachable: bool,
    /// What it holds, when it answered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spaces: Vec<Space>,
    /// Why not, when it did not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One space a CLI offers.
///
/// The page renders this before anything is replicated, so `subject` is
/// the only field it can rely on: a peer that knows its spaces by DID
/// alone has no name to give, and the display name on a space's own
/// content branch is unreadable until the space is opened.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Space {
    /// The space's subject DID.
    pub subject: String,
    /// What the CLI calls it locally, when it calls it anything.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Inventory {
    /// No CLI answered, and this is why.
    pub fn unreachable(detail: impl Into<String>) -> Self {
        Self {
            reachable: false,
            spaces: Vec::new(),
            detail: Some(detail.into()),
        }
    }

    /// A CLI answered with these.
    pub fn from_offers(offers: Vec<dialog_effects::peer::Offer>) -> Self {
        Self {
            reachable: true,
            spaces: offers
                .into_iter()
                .map(|offer| Space {
                    subject: offer.subject.to_string(),
                    name: offer.name,
                })
                .collect(),
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

/// How the operator's iroh site gets a channel.
///
/// The site is built when the worker starts and the channel cannot
/// exist then: it rides a carrier a page opens later, through
/// [`handle_carrier`]. So the operator is handed this instead of a
/// channel, and the endpoint is whatever [`Reach`] a page has since
/// caused to be bound.
///
/// The same `Reach` the status probe uses, deliberately — one endpoint
/// for this worker, not one per purpose. Its key is this peer's name,
/// and a second endpoint would be a second peer that every delegation
/// already minted names a stranger.
///
/// Before any page has dialed there is nothing to connect to, and
/// saying so is the whole answer: `Iroh` does not remember a failed
/// connect, so the next exchange after a carrier lands succeeds without
/// anything being rebuilt.
#[derive(Clone)]
pub struct CarrierConnect {
    reach: Arc<Lazy>,
}

impl CarrierConnect {
    /// Connect through whatever endpoint `reach` holds.
    pub fn new(reach: Arc<Lazy>) -> Self {
        Self { reach }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_iroh_remote::channel::Connect for CarrierConnect {
    async fn connect(
        &self,
    ) -> Result<
        Arc<dyn dialog_iroh_remote::channel::Channel>,
        dialog_iroh_remote::channel::ChannelError,
    > {
        let Some(reach) = self.reach.get() else {
            return Err(dialog_iroh_remote::channel::ChannelError::Unreachable {
                peer: "a local tonk".into(),
                detail: "no page has opened a carrier to a local tonk yet".into(),
            });
        };
        Ok(Arc::new(dialog_iroh_remote::transport::IrohChannel::new(
            reach.endpoint.clone(),
        )))
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

/// Ask the CLI what spaces it holds.
///
/// The same exchange as [`status`] with a different effect, and the same
/// treatment of failure: a peer that cannot be reached is an answer, not
/// an error status.
pub async fn spaces(
    reach: &Reach,
    id: iroh::EndpointId,
    phrase: &str,
    subject: dialog_capability::Did,
    operator: &crate::worker::DefaultOperator,
) -> Inventory {
    use dialog_capability::{Fork, ForkInvocation, Provider, SiteFork, Subject};
    use dialog_effects::Use;
    use dialog_effects::peer::{Peer, Spaces};
    use dialog_iroh_remote::site::{Iroh, IrohFork};

    let site = Iroh::new(dialog_iroh_remote::transport::IrohChannel::new(
        reach.endpoint.clone(),
    ));

    let ask = Subject::from(subject)
        .attenuate(Use)
        .attenuate(Peer)
        .attenuate(Spaces);
    let fork: IrohFork<Spaces> = Fork::<Iroh, _>::new(ask, reach.peer(id, phrase)).into();

    let invocation = match fork.authorize(operator).await {
        Ok(invocation) => invocation,
        Err(error) => {
            return Inventory::unreachable(format!("could not sign the request: {error}"));
        }
    };

    match Provider::<ForkInvocation<Iroh, Spaces>>::execute(&site, invocation).await {
        Ok(offers) => Inventory::from_offers(offers),
        Err(error) => Inventory::unreachable(error.to_string()),
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

    // The *listener's* name, not this side's. A route is a way to reach
    // a peer, and the peer at the far end of this carrier is the `tonk`.
    // Registering it under this end's name is a route to ourselves,
    // which iroh never sends on: the carrier opens and nothing moves.
    let peer = tonk_rtc::transport::rendezvous_addr(
        tonk_rtc::rendezvous::RENDEZVOUS,
        tonk_rtc::rendezvous::Side::Listener,
    );
    tonk_rtc::transport::relay::attach(&reach.transport, peer, port);

    // The event the site cannot see. A remote that failed to connect
    // before this — because there was no carrier, which is the ordinary
    // state until now — is sitting in a backoff whose reason has just
    // stopped being true. Waiting it out would mean a page that dialed
    // and then watched nothing happen.
    {
        let iroh = state.read().await.iroh.clone();
        iroh.revive().await;
    }
    log!("cli: a carrier from {client:?} is now a route");
}

/// What both probe routes need before they can ask a peer anything, or
/// the sentence to answer with instead.
///
/// Two ways to have nothing to ask, and they are different states a
/// reader acts on differently: an address that does not parse is the
/// caller's mistake, and an absent carrier is nobody having dialed yet,
/// which is the ordinary condition before anyone tries.
struct Probe {
    reach: Arc<Reach>,
    peer: dialog_iroh_remote::site::IrohAddress,
    operator: crate::worker::DefaultOperator,
    subject: dialog_capability::Did,
}

impl Probe {
    async fn prepare(state: &crate::router::AppState, peer: &str) -> Result<Self, String> {
        let peer = peer
            .parse::<dialog_iroh_remote::site::IrohAddress>()
            .map_err(|error| format!("that is not a peer address: {error}"))?;

        let (reach, operator, subject) = {
            let tonk = state.read().await;
            (
                tonk.reach.clone(),
                tonk.operator.clone(),
                tonk.profile.did(),
            )
        };

        let reach = reach
            .get()
            .cloned()
            .ok_or("no page has opened a carrier to a local tonk yet")?;

        Ok(Self {
            reach,
            peer,
            operator,
            subject,
        })
    }
}

/// The profile branch the network page renders, and where the live CLI
/// observation is stamped.
const PROFILE_BRANCH: &str = "main";

/// Ask the local `tonk` who it is and what it holds.
///
/// A command rather than a route, per `commands-not-routes`. Asking is
/// a user action, and its outcome is a fact the network page already
/// subscribes to — so every tab showing that page updates, rather than
/// one caller reading one response body once. It is also the same ask
/// from a test or the CLI, which a route could not be.
///
/// Reaching a peer needs a carrier some page opened. A command that
/// arrives before one does stamps `unreachable` and says why; that is
/// the ordinary state before anybody has dialed, not a fault.
///
/// Target-agnostic: the ask runs anywhere, and only the overlay stamp
/// is browser-shaped — a native caller has no page to render for.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::ReachPeer> for crate::router::CommandEnv {
    async fn execute(&self, command: tonk_schema::command::ReachPeer) {
        let inventory = reach(self.state(), &command.peer.0).await;
        publish_inventory(self.state(), &inventory).await;
    }
}

/// The whole of the ask: parse the peer, find a carrier, invoke.
///
/// Shared by the command and by tests, so what a test exercises is what
/// a click runs.
pub async fn reach(state: &crate::router::AppState, peer: &str) -> Inventory {
    let probe = match Probe::prepare(state, peer).await {
        Ok(probe) => probe,
        Err(detail) => return Inventory::unreachable(detail),
    };

    spaces(
        &probe.reach,
        *probe.peer.endpoint(),
        tonk_rtc::rendezvous::RENDEZVOUS,
        probe.subject,
        &probe.operator,
    )
    .await
}

/// Stamp an inventory answer as the live peer observation.
///
/// Overlay, not a commit: a running CLI is an observation with the
/// lifetime of a session, and committing it would replicate "this
/// laptop had a CLI up" to every other device on the profile. The
/// singleton key means asserting supersedes rather than accumulates, so
/// the page always folds to the latest.
///
/// Off wasm there is no page to render for, so this is the whole of it.
pub async fn publish_inventory(state: &crate::router::AppState, inventory: &Inventory) {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        use std::sync::Arc;
        use tonk_common::log;
        use tonk_schema::Peer;

        let tonk = state.read().await;
        let session = match tonk
            .reactor
            .profile_repository()
            .branch(PROFILE_BRANCH)
            .acquire(&tonk.operator)
            .await
        {
            Ok(session) => session,
            Err(error) => {
                log!("publish_inventory: could not acquire the profile branch: {error}");
                return;
            }
        };

        // `Peer` carries only the status, so the row still resolves when
        // the CLI is down. The identity fields ride in their own concept
        // because a required field that is sometimes absent makes the
        // whole row unresolvable.
        if inventory.reachable {
            session.state.assert_overlay(Peer::reachable());
        } else {
            session.state.assert_overlay(Peer::unreachable());
        }
        tonk.reactor.schedule_poll(Arc::clone(&session.state));
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    let _ = (state, inventory);
}
