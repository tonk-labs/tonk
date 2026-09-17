//! A signed dialog invocation, performed over a WebRTC data channel.
//!
//! The two halves of the peer remote have been built separately and
//! this is where they meet. `dialog-iroh-remote` supplies the protocol
//! — sign an invocation, send it, verify it, perform it — over an
//! `iroh::Endpoint` it does not build. This crate supplies an endpoint
//! whose only route is a data channel. Neither knows about the other,
//! and the whole bet is that they compose on the `Endpoint` type alone.
//!
//! So the assertion is deliberately small and end to end: a block put
//! by a browser-shaped peer lands in a CLI-shaped peer's store, having
//! been signed by a real operator, carried as a `ctn-v1` container over
//! QUIC over SCTP over DTLS, and verified against a real proof chain at
//! the far end. There is no UDP between the two and no relay.
//!
//! Native only, like its sibling: both peers are `webrtc-rs` here. The
//! browser half swaps `web-sys` channels underneath, which is the port
//! this test is meant to de-risk rather than perform.

#![cfg(all(feature = "iroh", not(target_arch = "wasm32")))]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use dialog_capability::{Fork, ForkInvocation, Provider, SiteFork, Subject};
use dialog_common::Buffer;
use dialog_did_web::{CachingResolver, WebResolver};
use dialog_effects::Use;
use dialog_effects::archive::{self, ArchiveError};
use dialog_iroh_remote::helpers::Volatile;
use dialog_iroh_remote::serve::Responder;
use dialog_iroh_remote::site::{Iroh, IrohAddress, IrohFork};
use dialog_iroh_remote::transport::{ALPN, IrohChannel, accept};
use dialog_operator::helpers::test_operator_with_profile;
use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, SecretKey, TransportAddr};
use tonk_rtc::transport::{WebRtcTransport, attach, datagram_channel};
use webrtc::api::APIBuilder;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::setting_engine::SettingEngine;
use webrtc::data_channel::RTCDataChannel;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;

const PATIENCE: Duration = Duration::from_secs(20);

/// Two peer connections wired to each other over loopback.
async fn pair() -> Result<(Arc<RTCPeerConnection>, Arc<RTCPeerConnection>)> {
    let build = || async {
        let mut settings = SettingEngine::default();
        // Without this there is no 127.0.0.1 candidate and two peers on
        // one machine have nothing to pair on.
        settings.set_include_loopback_candidate(true);
        anyhow::Ok(Arc::new(
            APIBuilder::new()
                .with_media_engine(MediaEngine::default())
                .with_setting_engine(settings)
                .build()
                .new_peer_connection(RTCConfiguration::default())
                .await?,
        ))
    };
    let (left, right) = (build().await?, build().await?);

    // A channel created before the offer is what puts an
    // `m=application` section in it; without one there is nothing to
    // answer.
    let _seed = datagram_channel(&left, "seed").await?;

    let offer = left.create_offer(None).await?;
    let mut gathered = left.gathering_complete_promise().await;
    left.set_local_description(offer).await?;
    let _ = tokio::time::timeout(Duration::from_secs(5), gathered.recv()).await;
    right
        .set_remote_description(left.local_description().await.expect("a local offer"))
        .await?;

    let answer = right.create_answer(None).await?;
    let mut gathered = right.gathering_complete_promise().await;
    right.set_local_description(answer).await?;
    let _ = tokio::time::timeout(Duration::from_secs(5), gathered.recv()).await;
    left.set_remote_description(right.local_description().await.expect("a local answer"))
        .await?;

    Ok((left, right))
}

/// Resolve once the channel is open.
async fn opened(channel: &Arc<RTCDataChannel>) -> Result<()> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let tx = Arc::new(Mutex::new(Some(tx)));
    channel.on_open(Box::new(move || {
        if let Some(tx) = tx.lock().ok().and_then(|mut slot| slot.take()) {
            let _ = tx.send(());
        }
        Box::pin(async {})
    }));
    tokio::time::timeout(PATIENCE, rx).await??;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_put_signed_in_one_peer_is_performed_in_another() -> Result<()> {
    let (left, right) = pair().await?;

    let dialer_rtc = WebRtcTransport::new(b"dialer");
    let listener_rtc = WebRtcTransport::new(b"listener");

    let dialing = datagram_channel(&left, "dialog").await?;
    let (accepted, inbound) = tokio::sync::oneshot::channel();
    let accepted = Arc::new(Mutex::new(Some(accepted)));
    right.on_data_channel(Box::new(move |channel: Arc<RTCDataChannel>| {
        let accepted = accepted.clone();
        Box::pin(async move {
            if channel.label() == "dialog"
                && let Some(tx) = accepted.lock().ok().and_then(|mut slot| slot.take())
            {
                let _ = tx.send(channel);
            }
        })
    }));

    let accepting: Arc<RTCDataChannel> = tokio::time::timeout(PATIENCE, inbound).await??;
    opened(&dialing).await?;

    attach(&dialer_rtc, listener_rtc.local_addr(), dialing);
    attach(&listener_rtc, dialer_rtc.local_addr(), accepting);

    // `presets::Empty` means empty: no relay, no discovery, no crypto
    // provider. The data channel is the only way these two can reach
    // each other, which is what makes the result unambiguous.
    let listener_key = SecretKey::generate();
    let listener_id = listener_key.public();
    let listening = Endpoint::builder(presets::Empty)
        .crypto_provider(iroh::tls::default_provider())
        .secret_key(listener_key)
        // The ALPN is dialog's, not this test's: the peer being dialed
        // has to be speaking the remote protocol, not merely reachable.
        .alpns(vec![ALPN.to_vec()])
        .add_custom_transport(listener_rtc.clone())
        .bind()
        .await?;
    let dialing = Endpoint::builder(presets::Empty)
        .crypto_provider(iroh::tls::default_provider())
        .secret_key(SecretKey::generate())
        .add_custom_transport(dialer_rtc.clone())
        .bind()
        .await?;

    let responder = Arc::new(Responder::new(
        Volatile::default(),
        CachingResolver::new(WebResolver::new()),
    ));
    let serving = tokio::spawn(accept(listening.clone(), responder.clone()));

    // This is the whole composition: dialog's channel over an endpoint
    // it knows nothing about, reached at a WebRTC route.
    let site = Iroh::new(IrohChannel::new(dialing.clone()));
    let peer = IrohAddress::from(EndpointAddr {
        id: listener_id,
        addrs: [TransportAddr::Custom(listener_rtc.local_addr())]
            .into_iter()
            .collect(),
    });

    let (operator, profile) = test_operator_with_profile().await;
    let bytes = b"signed here, stored across a data channel".to_vec();
    let put = Subject::from(profile.did())
        .attenuate(Use)
        .attenuate(archive::Archive)
        .attenuate(archive::Catalog::new("blocks"))
        .invoke(archive::Put::new(Buffer::from(bytes.clone())));

    let fork: IrohFork<archive::Put> = Fork::<Iroh, _>::new(put, peer).into();
    let invocation = fork.authorize(&operator).await.expect("authorized");
    let stored: Result<(), ArchiveError> = tokio::time::timeout(
        PATIENCE,
        Provider::<ForkInvocation<Iroh, archive::Put>>::execute(&site, invocation),
    )
    .await?;
    stored.expect("the peer verifies and performs the put");

    assert_eq!(
        responder
            .store()
            .get(Buffer::from(bytes.clone()).blake3_hash()),
        Some(bytes),
        "the block crossed a WebRTC data channel and landed in the peer's store"
    );

    dialing.close().await;
    listening.close().await;
    let _ = tokio::time::timeout(Duration::from_secs(5), serving).await;
    Ok(())
}
