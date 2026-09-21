//! Carriers are a transport dependency, not a prerequisite UI ceremony.
//! This context deliberately does not hold or re-lock `AppState`: repository
//! operations can request a channel while holding its read or write guard.

use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use super::{Lazy, Reach, deadline};
use crate::router::{ClientId, ClientRegistry};
use dialog_iroh_remote::{
    channel::{Channel, ChannelError, Transfer},
    site::IrohAddress,
    transport::IrohChannel,
};

const MAX_PEERS: usize = 4;
const MAX_WAITERS: usize = 64;
const DIAL_SECONDS: u64 = 30;
const IO_SECONDS: u64 = 30;

pub(super) struct Context {
    pub profile: dialog_operator::Profile,
    clients: ClientRegistry,
    epoch: Arc<AtomicU64>,
    generation: u64,
    valid: AtomicBool,
}

impl Context {
    pub fn current(&self) -> bool {
        self.valid.load(Ordering::Acquire) && self.epoch.load(Ordering::Acquire) == self.generation
    }

    pub async fn client_current(&self, id: &ClientId) -> bool {
        if !self.current() {
            return false;
        }
        let mut clients = self.clients.write().await;
        if !self.current() {
            return false;
        }
        let client = clients.entry(id.clone()).or_default();
        match client.context_generation {
            Some(bound) => bound == self.generation,
            None => {
                client.context_generation = Some(self.generation);
                true
            }
        }
    }
}

#[derive(Default)]
struct Peer {
    gate: tokio::sync::Mutex<()>,
    connected: Mutex<Option<Arc<Connected>>>,
}

/// The connection pool and carrier lease have exactly the same generation.
/// Replacing a carrier discards the old QUIC pool instead of reusing a cached
/// connection whose dead datagram route has not timed out yet.
pub(super) struct Connected {
    uri: String,
    pub lease: tonk_rtc::transport::Inbound,
    pub channel: IrohChannel,
}

impl Drop for Connected {
    fn drop(&mut self) {
        self.lease.detach();
    }
}

pub(super) struct Demand {
    context: Mutex<Option<Arc<Context>>>,
    peers: Mutex<BTreeMap<String, Arc<Peer>>>,
    waiters: tokio::sync::Semaphore,
    streams: Arc<tokio::sync::Semaphore>,
}

impl Default for Demand {
    fn default() -> Self {
        Self {
            context: Mutex::new(None),
            peers: Mutex::new(BTreeMap::new()),
            waiters: tokio::sync::Semaphore::new(MAX_WAITERS),
            streams: Arc::new(tokio::sync::Semaphore::new(MAX_WAITERS)),
        }
    }
}

impl Lazy {
    pub(super) fn connected(&self, peer: &IrohAddress) -> Option<Arc<Connected>> {
        self.demand
            .peers
            .lock()
            .expect("carrier peers")
            .get(&peer.did().to_string())?
            .connected
            .lock()
            .expect("carrier connection")
            .as_ref()
            .filter(|connected| connected.uri == peer.to_uri() && connected.lease.is_current())
            .cloned()
    }

    /// Bind after all lifecycle fields are final, including after a profile
    /// promotion. Old contexts and in-flight transfers fail closed on rebind.
    pub(crate) fn bind_context(&self, tonk: &crate::worker::TonkState) {
        let context = Arc::new(Context {
            profile: tonk.profile.clone(),
            clients: tonk.clients.clone(),
            epoch: tonk.context_generation.clone(),
            generation: tonk.context_generation.load(Ordering::Acquire),
            valid: AtomicBool::new(true),
        });
        if let Some(old) = self
            .demand
            .context
            .lock()
            .expect("carrier context")
            .replace(context)
        {
            old.valid.store(false, Ordering::Release);
        }
        for peer in self.demand.peers.lock().expect("carrier peers").values() {
            if let Some(connected) = peer.connected.lock().expect("carrier connection").take() {
                connected.lease.detach();
            }
        }
    }
}

impl Demand {
    fn context(&self) -> Result<Arc<Context>, String> {
        self.context
            .lock()
            .expect("carrier context")
            .clone()
            .filter(|context| context.current())
            .ok_or_else(|| "the carrier belongs to an inactive profile; reload this page".into())
    }

    fn peer(&self, id: &str) -> Result<Arc<Peer>, String> {
        let mut peers = self.peers.lock().expect("carrier peers");
        if let Some(peer) = peers.get(id) {
            return Ok(peer.clone());
        }
        // Keep a slot while a request is using it, even before it has a lease.
        peers.retain(|_, peer| {
            Arc::strong_count(peer) > 1
                || peer
                    .connected
                    .lock()
                    .expect("carrier connection")
                    .as_ref()
                    .is_some_and(|connected| connected.lease.is_current())
        });
        if peers.len() >= MAX_PEERS {
            return Err("this worker already carries four peers".into());
        }
        let peer = Arc::new(Peer::default());
        peers.insert(id.to_owned(), peer.clone());
        Ok(peer)
    }
}

/// The same acquisition path serves explicit discovery and saved remotes.
/// Queueing, page selection and dial setup together have a fixed deadline.
pub(super) async fn acquire(
    owner: &Arc<Lazy>,
    peer: &IrohAddress,
    origin: Option<&ClientId>,
) -> Result<(Arc<Context>, Arc<Connected>), String> {
    let _permit = owner
        .demand
        .waiters
        .try_acquire()
        .map_err(|_| "too many pending peer requests")?;
    deadline(
        async {
            let context = owner.demand.context()?;
            let (validated, address) = super::peer_route(&peer.to_uri())?;
            let state = owner.demand.peer(&peer.did().to_string())?;
            let _dial = state.gate.lock().await;
            if !context.current() {
                return Err("profile changed while waiting for a carrier".into());
            }
            let uri = validated.to_uri();
            if let Some(connected) = state
                .connected
                .lock()
                .expect("carrier connection")
                .as_ref()
                .filter(|connected| connected.uri == uri && connected.lease.is_current())
                .cloned()
            {
                return Ok((context, connected));
            }
            let reach = owner
                .get_or_try_init(|| async {
                    let key = super::super::peer_identity::peer_key(context.profile.signer())
                        .await
                        .map_err(|e| e.to_string())?;
                    Reach::bind(key).await.map(Arc::new)
                })
                .await?
                .clone();
            // Explicitly retire the old route before opening a replacement. A
            // failed replacement leaves a retryable disconnection, never fallback.
            if let Some(old) = state.connected.lock().expect("carrier connection").take() {
                old.lease.detach();
            }
            let lease =
                super::carrier::connect(&context, &reach, origin, &validated, &address).await?;
            let connected = Arc::new(Connected {
                uri,
                lease,
                channel: IrohChannel::new(reach.endpoint.clone()),
            });
            if !context.current() {
                return Err("profile changed while opening a carrier".into());
            }
            *state.connected.lock().expect("carrier connection") = Some(connected.clone());
            Ok((context, connected))
        },
        DIAL_SECONDS,
    )
    .await?
}

pub(super) struct DemandChannel(pub Arc<Lazy>);

fn unreachable(peer: &IrohAddress, detail: String) -> ChannelError {
    ChannelError::Unreachable {
        peer: peer.to_string(),
        detail,
    }
}

#[async_trait::async_trait(?Send)]
impl Channel for DemandChannel {
    async fn exchange(
        &self,
        peer: &IrohAddress,
        request: Vec<u8>,
    ) -> Result<Vec<u8>, ChannelError> {
        let mut transfer = self.open(peer, request).await?;
        transfer.finish().await?;
        dialog_iroh_remote::wire::read_frame("response", transfer.as_mut()).await
    }

    async fn open(
        &self,
        peer: &IrohAddress,
        request: Vec<u8>,
    ) -> Result<Box<dyn Transfer>, ChannelError> {
        let permit = self
            .0
            .demand
            .streams
            .clone()
            .try_acquire_owned()
            .map_err(|_| unreachable(peer, "too many open peer streams".into()))?;
        let (context, connected) = acquire(&self.0, peer, None)
            .await
            .map_err(|e| unreachable(peer, e))?;
        let opened = deadline(connected.channel.open(peer, request), IO_SECONDS)
            .await
            .map_err(|e| unreachable(peer, e))
            .and_then(|result| result);
        let inner = match opened {
            Ok(inner) => inner,
            Err(error) => {
                // Page heartbeats do not prove the CLI is still there. Retire
                // this generation so a later repository retry can redial.
                connected.lease.detach();
                return Err(error);
            }
        };
        let transfer = GuardedTransfer {
            context,
            connected,
            inner: Some(inner),
            peer: peer.to_string(),
            _permit: permit,
        };
        transfer.check()?;
        Ok(Box::new(transfer))
    }
}

/// Do not replay an interrupted mutation. The repository must reconcile its
/// CAS state before retrying. These guards only bound I/O and session lifetime.
struct GuardedTransfer {
    context: Arc<Context>,
    connected: Arc<Connected>,
    inner: Option<Box<dyn Transfer>>,
    peer: String,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl GuardedTransfer {
    fn interrupted(&self, detail: impl Into<String>) -> ChannelError {
        ChannelError::Interrupted {
            peer: self.peer.clone(),
            detail: detail.into(),
        }
    }
    fn check(&self) -> Result<(), ChannelError> {
        if self.inner.is_none() || !self.context.current() || !self.connected.lease.is_current() {
            return Err(self.interrupted("the carrier or profile changed during this exchange"));
        }
        Ok(())
    }
    fn settle<T>(
        &mut self,
        result: Result<Result<T, ChannelError>, String>,
    ) -> Result<T, ChannelError> {
        let result = result
            .map_err(|e| self.interrupted(e))
            .and_then(|result| result)
            .and_then(|value| self.check().map(|_| value));
        if result.is_err() {
            self.inner.take();
            self.connected.lease.detach();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_service_worker);

    #[dialog_common::test]
    async fn context_and_client_generation_fail_closed_without_relocking_app_state() {
        let state = crate::router::tests::test_state_without_root().await;
        let state = Arc::new(tokio::sync::RwLock::new(state));
        // A real exclusive guard: the demand context must not try to acquire it.
        let tonk = state.write().await;
        tonk.reach.bind_context(&tonk);
        let old = tonk.reach.demand.context().unwrap();
        let client = ClientId("old-page".into());
        assert!(old.client_current(&client).await);
        tonk.context_generation.fetch_add(1, Ordering::AcqRel);
        assert!(!old.current());
        assert!(!old.client_current(&client).await);
        assert!(tonk.reach.demand.context().is_err());
        tonk.reach.bind_context(&tonk);
        let current = tonk.reach.demand.context().unwrap();
        assert!(current.current());
        assert!(!current.client_current(&client).await);
        assert!(current.client_current(&ClientId("new-page".into())).await);
        tonk.reach.bind_context(&tonk);
        assert!(!current.current(), "rebind invalidates even the same epoch");
    }

    #[dialog_common::test]
    async fn peer_slots_and_pending_and_open_requests_are_bounded_and_reusable() {
        let demand = Demand::default();
        let mut held: Vec<_> = (0..MAX_PEERS)
            .map(|i| demand.peer(&format!("peer-{i}")).unwrap())
            .collect();
        assert!(Arc::ptr_eq(&held[0], &demand.peer("peer-0").unwrap()));
        assert!(demand.peer("one-too-many").is_err());
        held.remove(0);
        assert!(demand.peer("replacement").is_ok());
        let pending: Vec<_> = (0..MAX_WAITERS)
            .map(|_| demand.waiters.try_acquire().unwrap())
            .collect();
        assert!(demand.waiters.try_acquire().is_err());
        drop(pending);
        assert!(demand.waiters.try_acquire().is_ok());
        let streams: Vec<_> = (0..MAX_WAITERS)
            .map(|_| demand.streams.clone().try_acquire_owned().unwrap())
            .collect();
        assert!(demand.streams.clone().try_acquire_owned().is_err());
        drop(streams);
        assert!(demand.streams.clone().try_acquire_owned().is_ok());
    }

    struct RecordingTransfer {
        calls: Rc<Cell<usize>>,
        revoke: Option<Arc<AtomicU64>>,
    }
    #[async_trait::async_trait(?Send)]
    impl Transfer for RecordingTransfer {
        async fn send(&mut self, _: &[u8]) -> Result<(), ChannelError> {
            self.calls.set(self.calls.get() + 1);
            if let Some(epoch) = self.revoke.take() {
                epoch.fetch_add(1, Ordering::AcqRel);
            }
            Ok(())
        }
        async fn finish(&mut self) -> Result<(), ChannelError> {
            self.send(&[]).await
        }
        async fn read_exact(&mut self, len: usize) -> Result<Vec<u8>, ChannelError> {
            self.send(&[]).await?;
            Ok(vec![0; len])
        }
        async fn recv(&mut self) -> Result<Option<Vec<u8>>, ChannelError> {
            self.send(&[]).await?;
            Ok(None)
        }
    }

    #[dialog_common::test]
    async fn existing_transfers_stop_on_route_or_profile_change_and_never_replay() {
        let tonk = crate::router::tests::test_state_without_root().await;
        let reach = Reach::bind(iroh::SecretKey::from_bytes(&[11; 32]))
            .await
            .unwrap();
        let route = reach.transport.local_addr();
        for during_io in [false, true] {
            tonk.reach.bind_context(&tonk);
            let context = tonk.reach.demand.context().unwrap();
            let port = reach.transport.attach(route.clone());
            let lease = port.inbound;
            let connected = Arc::new(Connected {
                uri: "fixture".into(),
                lease: lease.clone(),
                channel: IrohChannel::new(reach.endpoint.clone()),
            });
            let calls = Rc::new(Cell::new(0));
            let mut transfer = GuardedTransfer {
                context,
                connected,
                peer: "fixture".into(),
                inner: Some(Box::new(RecordingTransfer {
                    calls: calls.clone(),
                    revoke: during_io.then(|| tonk.context_generation.clone()),
                })),
                _permit: tonk
                    .reach
                    .demand
                    .streams
                    .clone()
                    .try_acquire_owned()
                    .unwrap(),
            };
            if !during_io {
                lease.detach();
            }
            assert!(matches!(
                transfer.send(b"mutation").await,
                Err(ChannelError::Interrupted { .. })
            ));
            assert!(transfer.finish().await.is_err());
            assert!(transfer.recv().await.is_err());
            assert_eq!(
                calls.get(),
                usize::from(during_io),
                "never replay a possibly applied mutation"
            );
        }
        reach.endpoint.close().await;
    }
}

#[async_trait::async_trait(?Send)]
impl Transfer for GuardedTransfer {
    async fn send(&mut self, bytes: &[u8]) -> Result<(), ChannelError> {
        self.check()?;
        let result = deadline(self.inner.as_mut().unwrap().send(bytes), IO_SECONDS).await;
        self.settle(result)
    }
    async fn finish(&mut self) -> Result<(), ChannelError> {
        self.check()?;
        let result = deadline(self.inner.as_mut().unwrap().finish(), IO_SECONDS).await;
        self.settle(result)
    }
    async fn read_exact(&mut self, len: usize) -> Result<Vec<u8>, ChannelError> {
        self.check()?;
        let result = deadline(self.inner.as_mut().unwrap().read_exact(len), IO_SECONDS).await;
        self.settle(result)
    }
    async fn recv(&mut self) -> Result<Option<Vec<u8>>, ChannelError> {
        self.check()?;
        let result = deadline(self.inner.as_mut().unwrap().recv(), IO_SECONDS).await;
        self.settle(result)
    }
}
