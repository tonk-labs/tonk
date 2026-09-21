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
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
mod attach;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod carrier;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod demand;

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
#[derive(Default)]
pub struct Lazy {
    endpoint: tokio::sync::OnceCell<Arc<Reach>>,
    attempt: AtomicU64,
    /// Only the latest successful, authenticated inventory can be selected.
    offers: std::sync::Mutex<Option<(String, Vec<Space>)>>,
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    demand: demand::Demand,
}

impl std::ops::Deref for Lazy {
    type Target = tokio::sync::OnceCell<Arc<Reach>>;
    fn deref(&self) -> &Self::Target {
        &self.endpoint
    }
}

/// One connect observation. An old command may finish, but it must not
/// overwrite the result of a newer command or a replacement profile.
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    allow(dead_code)
)]
#[derive(Clone)]
struct Attempt {
    owner: Arc<Lazy>,
    generation: u64,
}

impl Attempt {
    fn begin(owner: Arc<Lazy>) -> Self {
        let generation = owner.attempt.fetch_add(1, Ordering::SeqCst) + 1;
        Self { owner, generation }
    }

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    async fn while_current<T>(
        &self,
        future: impl std::future::Future<Output = T>,
    ) -> Result<T, String> {
        use futures_util::future::{Either, select};
        if !self.is_current(&self.owner) {
            return Err("connect attempt superseded".into());
        }
        let cancelled = async {
            while self.is_current(&self.owner) {
                let _ = crate::r#async::sleep(web_time::Duration::from_millis(100)).await;
            }
        };
        match select(Box::pin(future), Box::pin(cancelled)).await {
            Either::Left((result, _)) if self.is_current(&self.owner) => Ok(result),
            _ => Err("connect attempt superseded".into()),
        }
    }

    #[cfg_attr(
        not(all(target_arch = "wasm32", target_os = "unknown")),
        allow(dead_code)
    )]
    fn is_current(&self, owner: &Arc<Lazy>) -> bool {
        Arc::ptr_eq(&self.owner, owner) && owner.attempt.load(Ordering::SeqCst) == self.generation
    }
}

/// Parse the exact transport route printed by the CLI. A bare identity
/// cannot locate a peer offline; never silently substitute the global
/// rendezvous or race other local listeners for it.
pub(super) fn peer_route(
    uri: &str,
) -> Result<(dialog_iroh_remote::site::IrohAddress, tonk_rtc::Address), String> {
    let peer: dialog_iroh_remote::site::IrohAddress = uri
        .trim()
        .parse()
        .map_err(|error| format!("that is not a peer address: {error}"))?;
    if peer.addr().addrs.len() != 1 {
        return Err("paste the complete URI from `tonk rtc serve`, with one WebRTC route".into());
    }
    let Some(iroh::TransportAddr::Custom(route)) = peer.addr().addrs.iter().next() else {
        return Err("this local connection requires a WebRTC route".into());
    };
    if route.id() != tonk_rtc::transport::TRANSPORT_ID {
        return Err("the peer URI names an unsupported custom transport".into());
    }
    let encoded = std::str::from_utf8(route.data()).map_err(|_| "the WebRTC route is not UTF-8")?;
    let address = tonk_rtc::Address::decode(encoded).map_err(|error| error.to_string())?;
    if !address.is_loopback() {
        return Err(
            "this milestone only connects to loopback peers; run a local `tonk rtc serve`".into(),
        );
    }
    Ok((peer, address))
}

impl Reach {
    /// Bind an endpoint over a fresh transport, as this device profile.
    ///
    /// The key is the profile's own, held durably in its credential
    /// store, not minted per run. It used to be ephemeral on the
    /// argument that this side only ever dials, and what a CLI verifies
    /// is the invocation's proof chain rather than who carried it. That
    /// held while the browser was only a client; it stops holding the
    /// moment a peer is something you can *add as a remote*, because a
    /// remote records an identity and has to keep reaching it.
    pub async fn bind(key: iroh::SecretKey) -> Result<Self, String> {
        let transport =
            tonk_rtc::transport::WebRtcTransport::new(tonk_rtc::rendezvous::transport_tag(
                tonk_rtc::rendezvous::RENDEZVOUS,
                tonk_rtc::rendezvous::Side::Dialer,
            ));

        let endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::Empty)
            .crypto_provider(iroh::tls::default_provider())
            .secret_key(key)
            .add_custom_transport(transport.clone())
            .bind()
            .await
            .map_err(|error| format!("could not bind an endpoint: {error}"))?;

        Ok(Self {
            transport,
            endpoint,
        })
    }
}

/// How the operator's iroh site gets a channel.
///
/// The site is built when the worker starts and the channel cannot
/// exist then: it rides a carrier a page opens later, through
/// a worker-initiated carrier request. So the operator is handed this instead of a
/// channel, and the endpoint is whatever [`Reach`] a page has since
/// caused to be bound.
///
/// The same `Reach` the status probe uses, deliberately — one endpoint
/// for this worker, not one per purpose. Its key is this peer's name,
/// and a second endpoint would be a second peer that every delegation
/// already minted names a stranger.
///
/// Browser channels acquire an addressed carrier on first use and after
/// disconnection. This also works for saved remotes after a worker restart.
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
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        return Ok(Arc::new(demand::DemandChannel(self.reach.clone())));
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        {
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
    peer: dialog_iroh_remote::site::IrohAddress,
    profile: &dialog_operator::Profile,
    operator: &crate::worker::DefaultOperator,
) -> Status {
    status_on(
        dialog_iroh_remote::transport::IrohChannel::new(reach.endpoint.clone()),
        peer,
        profile,
        operator,
    )
    .await
}

async fn status_on(
    channel: dialog_iroh_remote::transport::IrohChannel,
    peer: dialog_iroh_remote::site::IrohAddress,
    profile: &dialog_operator::Profile,
    operator: &crate::worker::DefaultOperator,
) -> Status {
    use dialog_capability::{Fork, ForkInvocation, Provider, Subject};
    use dialog_effects::Use;
    use dialog_effects::peer::{Hello, Peer};
    use dialog_iroh_remote::site::{Iroh, IrohAuthorization};

    let site = Iroh::new(channel);

    let hello = Subject::from(profile.did())
        .attenuate(Use)
        .attenuate(Peer)
        .attenuate(Hello);
    let signed = match profile
        .access()
        .claim(hello.clone())
        .invoke()
        .perform(operator)
        .await
    {
        Ok(invocation) => invocation,
        // Failing to sign is this side's problem, and saying "the CLI is
        // unreachable" would send someone looking in the wrong place.
        Err(error) => return Status::unreachable(format!("could not sign the request: {error}")),
    };
    let bytes = match dialog_ucan_core::Container::from(signed.chain()).into_bytes() {
        Ok(bytes) => bytes,
        Err(error) => return Status::unreachable(format!("could not encode the request: {error}")),
    };
    let invocation = Fork::<Iroh, _>::new(hello, peer).attest(IrohAuthorization::new(bytes));

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
    peer: dialog_iroh_remote::site::IrohAddress,
    profile: &dialog_operator::Profile,
    operator: &crate::worker::DefaultOperator,
) -> Inventory {
    use dialog_capability::{Fork, ForkInvocation, Provider, Subject};
    use dialog_effects::Use;
    use dialog_effects::peer::{Peer, Spaces};
    use dialog_iroh_remote::site::{Iroh, IrohAuthorization};

    let site = Iroh::new(dialog_iroh_remote::transport::IrohChannel::new(
        reach.endpoint.clone(),
    ));

    let ask = Subject::from(profile.did())
        .attenuate(Use)
        .attenuate(Peer)
        .attenuate(Spaces);
    // Public discovery needs no delegated authority. Sign as the profile for
    // its own subject: the server still performs ordinary UCAN verification,
    // but no session delegation needs a revocation service just to list names.
    let signed = match profile
        .access()
        .claim(ask.clone())
        .invoke()
        .perform(operator)
        .await
    {
        Ok(invocation) => invocation,
        Err(error) => {
            return Inventory::unreachable(format!("could not sign the request: {error}"));
        }
    };
    let bytes = match dialog_ucan_core::Container::from(signed.chain()).into_bytes() {
        Ok(bytes) => bytes,
        Err(error) => {
            return Inventory::unreachable(format!("could not encode the request: {error}"));
        }
    };
    let invocation = Fork::<Iroh, _>::new(ask, peer).attest(IrohAuthorization::new(bytes));

    match Provider::<ForkInvocation<Iroh, Spaces>>::execute(&site, invocation).await {
        Ok(offers) => Inventory::from_offers(offers),
        Err(error) => Inventory::unreachable(error.to_string()),
    }
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
    profile: dialog_operator::Profile,
}

impl Probe {
    async fn prepare(state: &crate::router::AppState, peer: &str) -> Result<Self, String> {
        let peer = peer
            .parse::<dialog_iroh_remote::site::IrohAddress>()
            .map_err(|error| format!("that is not a peer address: {error}"))?;

        let (reach, operator, profile) = {
            let tonk = state.read().await;
            (
                tonk.reach.clone(),
                tonk.operator.clone(),
                tonk.profile.clone(),
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
            profile,
        })
    }
}

/// The profile branch the network page renders, and where the live CLI
/// observation is stamped.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const PROFILE_BRANCH: &str = "main";

/// Ask the local `tonk` who it is and what it holds.
///
/// A command rather than a route, per `commands-not-routes`. Asking is
/// a user action, and its outcome is a fact the network page already
/// subscribes to — so every tab showing that page updates, rather than
/// one caller reading one response body once. It is also the same ask
/// from a test or the CLI, which a route could not be.
///
/// On the web, the command requests an explicitly addressed carrier from
/// a controlled page before probing. Each attempt owns its observations,
/// so an older result cannot overwrite a newer attempt or profile.
///
/// Target-agnostic: the ask runs anywhere, and only the overlay stamp
/// is browser-shaped — a native caller has no page to render for.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::ReachPeer> for crate::router::CommandEnv {
    async fn execute(&self, command: tonk_schema::command::ReachPeer) {
        let attempt = Attempt::begin(self.state().read().await.reach.clone());
        let peer = command.peer.0.trim();
        let (address, route) = match peer_route(peer) {
            Ok(plan) => plan,
            Err(detail) => {
                publish_observation(
                    self.state(),
                    &attempt,
                    peer,
                    "peer:invalid",
                    &Inventory::unreachable(detail),
                )
                .await;
                return;
            }
        };
        publish_observation(
            self.state(),
            &attempt,
            peer,
            "peer:connecting",
            &Inventory::unreachable("opening a local carrier…"),
        )
        .await;
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        if let Err(detail) = attempt
            .while_current(demand::acquire(&attempt.owner, &address, self.client()))
            .await
            .and_then(|result| result)
        {
            publish_observation(
                self.state(),
                &attempt,
                peer,
                tonk_schema::Peer::UNREACHABLE,
                &Inventory::unreachable(detail),
            )
            .await;
            return;
        }
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            let _ = route;
            self.state().read().await.iroh.revive().await;
        }
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        let _ = (address, route);
        let inventory = match deadline(reach(self.state(), peer), 15).await {
            Ok(inventory) => inventory,
            Err(detail) => Inventory::unreachable(detail),
        };
        let status = if inventory.reachable {
            tonk_schema::Peer::REACHABLE
        } else {
            tonk_schema::Peer::UNREACHABLE
        };
        publish_observation(self.state(), &attempt, peer, status, &inventory).await;
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        if inventory.reachable {
            watch_observation(self.state().clone(), attempt, peer.to_owned());
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn watch_observation(state: crate::router::AppState, attempt: Attempt, uri: String) {
    wasm_bindgen_futures::spawn_local(async move {
        loop {
            let _ = crate::r#async::sleep(web_time::Duration::from_secs(3)).await;
            let owner = state.read().await.reach.clone();
            if !attempt.is_current(&owner) {
                break;
            }
            let connected = peer_route(&uri)
                .ok()
                .and_then(|(peer, _)| owner.connected(&peer));
            let alive = if let Some(connected) = connected {
                // A page can keep answering MessagePort heartbeats after the
                // CLI exits. A bounded, read-only authenticated hello checks
                // the remote too, without opening a replacement carrier or
                // replaying repository work. Keep the same connection pool.
                let alive = match Probe::prepare(&state, &uri).await {
                    Ok(probe) => deadline(
                        status_on(
                            connected.channel.clone(),
                            probe.peer,
                            &probe.profile,
                            &probe.operator,
                        ),
                        5,
                    )
                    .await
                    .is_ok_and(|status| status.reachable),
                    Err(_) => false,
                };
                if !connected.lease.is_current() {
                    // A saved-remote request may have replaced this carrier
                    // while the hello was in flight. Observe its generation
                    // on the next beat; do not overwrite it with a late error.
                    continue;
                }
                if !alive && attempt.is_current(&owner) {
                    connected.lease.detach();
                }
                alive
            } else {
                false
            };
            if !alive {
                publish_observation(
                    &state,
                    &attempt,
                    &uri,
                    tonk_schema::Peer::UNREACHABLE,
                    &Inventory::unreachable("the carrier disconnected; connect again to retry"),
                )
                .await;
                break;
            }
        }
    });
}

async fn deadline<T>(
    future: impl std::future::Future<Output = T>,
    seconds: u64,
) -> Result<T, String> {
    use futures_util::future::{Either, select};
    match select(
        Box::pin(future),
        Box::pin(crate::r#async::sleep(web_time::Duration::from_secs(
            seconds,
        ))),
    )
    .await
    {
        Either::Left((result, _)) => Ok(result),
        Either::Right(_) => Err(
            "the peer did not answer before the connection deadline; retry when the CLI is running"
                .into(),
        ),
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

    spaces(&probe.reach, probe.peer, &probe.profile, &probe.operator).await
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
async fn publish_observation(
    state: &crate::router::AppState,
    attempt: &Attempt,
    address: &str,
    status: &str,
    inventory: &Inventory,
) {
    {
        let tonk = state.read().await;
        if !attempt.is_current(&tonk.reach) {
            return;
        }
        *tonk.reach.offers.lock().expect("peer offers lock") = inventory
            .reachable
            .then(|| (address.to_owned(), inventory.spaces.clone()));
    }
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        use std::sync::Arc;
        use tonk_common::log;
        use tonk_schema::Peer;

        let tonk = state.read().await;
        if !attempt.is_current(&tonk.reach) {
            return;
        }
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
        if !attempt.is_current(&tonk.reach) {
            return;
        }
        session.state.assert_overlay(Peer {
            this: Peer::entity(),
            status: tonk_schema::domain::peer::Status(status.parse().expect("static peer status")),
        });
        // Write empty values too: a failed retry must not retain the last
        // successful identity/count in its overlay.
        let subject = if inventory.reachable {
            address
                .parse::<dialog_iroh_remote::site::IrohAddress>()
                .map(|peer| peer.did().to_string())
                .unwrap_or_default()
        } else {
            String::new()
        };
        session
            .state
            .assert_overlay(tonk_schema::peer::PeerIdentity::new(
                subject,
                inventory.spaces.len() as u64,
            ));
        session.state.assert_overlay(tonk_schema::peer::PeerDetail {
            this: Peer::entity(),
            address: tonk_schema::domain::peer::Address(address.to_owned()),
            detail: tonk_schema::domain::peer::Detail(inventory.detail.clone().unwrap_or_else(
                || format!("connected; {} spaces offered", inventory.spaces.len()),
            )),
        });
        session
            .state
            .retain_overlay_entities(|entity| !entity.to_string().starts_with("state:cli/offer/"));
        for (index, space) in inventory.spaces.iter().enumerate() {
            session.state.assert_overlay(tonk_schema::peer::PeerOffer {
                this: format!("state:cli/offer/{index}")
                    .parse()
                    .expect("offer entity"),
                subject: tonk_schema::domain::peer_offer::Subject(space.subject.clone()),
                name: tonk_schema::domain::peer_offer::Name(
                    space.name.clone().unwrap_or_else(|| "unnamed space".into()),
                ),
                address: tonk_schema::domain::peer_offer::Address(address.to_owned()),
            });
        }
        tonk.reactor.schedule_poll(Arc::clone(&session.state));
        // Disconnect observations also originate from the detached watcher,
        // outside command dispatch's final poll flush. Publish them now so a
        // subscribed Network page cannot keep rendering a stale success.
        tonk.reactor.run_scheduled_polls(&tonk.operator).await;
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    let _ = (state, attempt, address, status, inventory);
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn uri(host: &str, port: u16) -> String {
        let route = tonk_rtc::Address {
            version: 1,
            candidates: vec![tonk_rtc::Candidate {
                host: host.into(),
                port,
            }],
            fingerprint: format!("sha-256 {}", ["AB"; 32].join(":")),
        };
        let transport = tonk_rtc::transport::WebRtcTransport::new(route.encode());
        dialog_iroh_remote::site::IrohAddress::new(iroh::EndpointAddr {
            id: iroh::SecretKey::from_bytes(&[7; 32]).public(),
            addrs: [iroh::TransportAddr::Custom(transport.local_addr())]
                .into_iter()
                .collect(),
        })
        .to_uri()
    }

    #[test]
    fn route_selection_preserves_the_endpoint_port_and_private_fingerprint() {
        let uri = uri("127.0.0.1", 45678);
        let (peer, route) = peer_route(&uri).unwrap();
        assert_eq!(peer.to_uri(), uri);
        assert_eq!(route.candidates[0].port, 45678);
        assert_eq!(
            route.fingerprint,
            format!("sha-256 {}", ["AB"; 32].join(":"))
        );
        assert!(peer_route(&peer.did().to_string()).is_err());
        assert!(peer_route(&self::uri("192.0.2.1", 45678)).is_err());
    }

    #[test]
    fn observations_are_owned_by_attempt_and_profile_generation() {
        let owner = Arc::new(Lazy::default());
        let old = Attempt::begin(owner.clone());
        assert!(old.is_current(&owner));
        let current = Attempt::begin(owner.clone());
        assert!(!old.is_current(&owner));
        assert!(current.is_current(&owner));
        assert!(!current.is_current(&Arc::new(Lazy::default())));
    }
}
