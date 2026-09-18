//! The worker side of the CLI relay, in a page, for the browser test.
//!
//! `tonk-worker` is where this lives in production, and it cannot be
//! built here: the app needs `trunk`, which this container has no copy
//! of. So the same three calls are exposed to JavaScript directly —
//! bind an endpoint over a `WebRtcTransport`, attach a transferred
//! `MessagePort` as a route, invoke `peer::Hello` over it.
//!
//! What that leaves untested is the service worker's own boundary: the
//! envelope dispatch and the `AppState` lookup. What it does test is the
//! part that has never run — a `MessagePort` carrying iroh datagrams to
//! a real `tonk`, and a signed invocation answered across it.

use std::sync::Arc;

use dialog_capability::{Fork, ForkInvocation, Provider, SiteFork, Subject};
use dialog_effects::Use;
use dialog_effects::peer::{Hello, Peer};
use dialog_iroh_remote::site::{Iroh, IrohFork};
use wasm_bindgen::prelude::*;

/// A bound endpoint, and the transport pages attach carriers to.
#[wasm_bindgen]
pub struct Reach {
    transport: Arc<tonk_rtc::transport::WebRtcTransport>,
    endpoint: iroh::Endpoint,
}

#[wasm_bindgen]
impl Reach {
    /// Bind an endpoint over a fresh transport, naming this side the
    /// dialer — the same name the CLI attaches its carrier under.
    pub async fn bind() -> Result<Reach, JsValue> {
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
            .map_err(|error| JsValue::from_str(&format!("bind: {error}")))?;

        Ok(Reach {
            transport,
            endpoint,
        })
    }

    /// Take a carrier the page opened and make it a route.
    pub fn attach(&self, port: web_sys::MessagePort) {
        tonk_rtc::transport::relay::attach(
            &self.transport,
            tonk_rtc::transport::rendezvous_addr(
                tonk_rtc::rendezvous::RENDEZVOUS,
                tonk_rtc::rendezvous::Side::Dialer,
            ),
            port,
        );
    }

    /// Ask the `tonk` named by `peer` who it is.
    ///
    /// Returns the three DIDs joined by spaces, which is enough for the
    /// test to assert on and keeps no serialization in the way of the
    /// thing being measured.
    pub async fn hello(&self, peer: String) -> Result<String, JsValue> {
        let address: dialog_iroh_remote::site::IrohAddress = peer
            .parse()
            .map_err(|error| JsValue::from_str(&format!("not a peer: {error}")))?;

        let address = dialog_iroh_remote::site::IrohAddress::from(iroh::EndpointAddr {
            id: *address.endpoint(),
            addrs: [iroh::TransportAddr::Custom(
                tonk_rtc::transport::rendezvous_addr(
                    tonk_rtc::rendezvous::RENDEZVOUS,
                    tonk_rtc::rendezvous::Side::Listener,
                ),
            )]
            .into_iter()
            .collect(),
        });

        let (operator, profile) = dialog_operator::helpers::test_operator_with_profile().await;
        let site = Iroh::new(dialog_iroh_remote::transport::IrohChannel::new(
            self.endpoint.clone(),
        ));

        let hello = Subject::from(profile.did())
            .attenuate(Use)
            .attenuate(Peer)
            .attenuate(Hello);
        let fork: IrohFork<Hello> = Fork::<Iroh, _>::new(hello, address).into();
        let invocation = fork
            .authorize(&operator)
            .await
            .map_err(|error| JsValue::from_str(&format!("authorize: {error}")))?;

        let greeting = Provider::<ForkInvocation<Iroh, Hello>>::execute(&site, invocation)
            .await
            .map_err(|error| JsValue::from_str(&format!("hello: {error}")))?;

        Ok(format!(
            "{} {} {}",
            greeting.subject, greeting.profile, greeting.operator
        ))
    }
}
