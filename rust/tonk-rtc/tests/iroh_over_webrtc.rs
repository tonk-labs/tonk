//! iroh running over a WebRTC data channel, with no UDP between the
//! peers and no relay.
//!
//! This is the proof that the local dial can be a *route* rather than a
//! separate code path: `TransportAddr::Custom` sits beside `Relay` and
//! `Ip` in one `EndpointAddr`, so a caller says
//! `endpoint.connect(id, alpn)` and never learns which one carried it.
//! Offline, this is the only one that can.
//!
//! Native only. The browser half is the same transport with `web-sys`
//! channels underneath, which is a port, not a redesign.

#![cfg(all(feature = "iroh", not(target_arch = "wasm32")))]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, SecretKey, TransportAddr};
use tonk_rtc::transport::{WebRtcTransport, attach, datagram_channel};
use webrtc::api::APIBuilder;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::setting_engine::SettingEngine;
use webrtc::data_channel::RTCDataChannel;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;

const ALPN: &[u8] = b"tonk/iroh-over-webrtc/1";
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
async fn iroh_streams_ride_a_webrtc_data_channel() -> Result<()> {
    let (left, right) = pair().await?;

    let dialer_rtc = WebRtcTransport::new(b"dialer");
    let listener_rtc = WebRtcTransport::new(b"listener");

    // One datagram channel, adopted by the far side.
    let dialing = datagram_channel(&left, "iroh").await?;
    let (accepted, inbound) = tokio::sync::oneshot::channel();
    let accepted = Arc::new(Mutex::new(Some(accepted)));
    right.on_data_channel(Box::new(move |channel: Arc<RTCDataChannel>| {
        let accepted = accepted.clone();
        Box::pin(async move {
            if channel.label() == "iroh"
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

    // Endpoints whose ONLY transport is that channel: no UDP between
    // them, no relay, no discovery. `presets::Empty` means empty, which
    // includes the crypto provider — hence supplying one by hand.
    let listener_key = SecretKey::generate();
    let listener_id = listener_key.public();
    let listener = Endpoint::builder(presets::Empty)
        .crypto_provider(iroh::tls::default_provider())
        .secret_key(listener_key)
        .alpns(vec![ALPN.to_vec()])
        .add_custom_transport(listener_rtc.clone())
        .bind()
        .await?;
    let dialer = Endpoint::builder(presets::Empty)
        .crypto_provider(iroh::tls::default_provider())
        .secret_key(SecretKey::generate())
        .add_custom_transport(dialer_rtc.clone())
        .bind()
        .await?;

    let serving = tokio::spawn(async move {
        let connection = listener
            .accept()
            .await
            .expect("an inbound connection")
            .await?;
        let (mut send, mut recv) = connection.accept_bi().await?;
        let asked = recv.read_to_end(64 * 1024).await?;
        send.write_all(format!("echo:{}", String::from_utf8_lossy(&asked)).as_bytes())
            .await?;
        send.finish()?;
        // Stay up until the peer has read the reply.
        connection.closed().await;
        anyhow::Ok(())
    });

    // The peer is named by its endpoint id; the WebRTC route is just one
    // address it can be reached on.
    let address = EndpointAddr {
        id: listener_id,
        addrs: [TransportAddr::Custom(listener_rtc.local_addr())]
            .into_iter()
            .collect(),
    };
    let connection = tokio::time::timeout(PATIENCE, dialer.connect(address, ALPN)).await??;

    let (mut send, mut recv) = connection.open_bi().await?;
    send.write_all(b"over a data channel").await?;
    send.finish()?;
    let reply = recv.read_to_end(64 * 1024).await?;
    assert_eq!(reply, b"echo:over a data channel");

    connection.close(0u32.into(), b"done");
    let _ = tokio::time::timeout(Duration::from_secs(5), serving).await;
    Ok(())
}
