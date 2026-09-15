//! Spike: can a browser dial this process with NO answer coming back?
//!
//! **Measured answer: not on `webrtc` 0.20.5 — but the gap is two short
//! strings, not a whole answer.** Findings are recorded below; this
//! program is kept so they can be re-measured against a later release.
//!
//! The premise of "publish an address and let peers dial it" is that
//! everything the browser needs is knowable in advance, so nothing has
//! to travel back. This process therefore runs ICE **lite** (hoping to
//! never need the browser's ICE password), **disables DTLS fingerprint
//! verification** (so it never needs the browser's certificate), uses
//! **static ICE credentials** (so its own half is publishable), and
//! feeds itself a **fabricated offer** whose ICE credentials and
//! fingerprint are placeholders. It prints the answer it produced; a
//! browser sets that as its remote description and dials.
//!
//! # What this established
//!
//! - **Chromium dials a `127.0.0.1` remote candidate without complaint.**
//!   Binding requests arrive at a loopback-bound port from an `http` and
//!   an `https` origin alike. This was the doubt that prompted
//!   `--bind`; it turned out not to be a constraint.
//!
//! - **The peer's `ice-ufrag` is required.** Username validation checks
//!   both halves (`local:remote`), so a placeholder is rejected:
//!   `ErrMismatchUsername expected(tonkdial:PLACEHOLD) actual(tonkdial:J7wc)`.
//!
//! - **The peer's `ice-pwd` is required too, because `set_lite(true)`
//!   does not suppress outgoing checks in `rtc-ice` 0.20.5.** Despite
//!   lite mode the agent calls `ping_candidate` and sends its own
//!   binding request, signed with the *remote* password; a wrong one
//!   draws a `401` and the pair never validates. This contradicts the
//!   setting's own documentation ("lite agents do not perform
//!   connectivity checks") and is worth reporting upstream. If it were
//!   fixed, only the ufrag would be needed.
//!
//! - **Neither the peer's candidates nor its fingerprint are needed.**
//!   A peer-reflexive candidate is formed from the source address of
//!   the first check, and inbound message integrity is verified with
//!   THIS side's own password — both knowable in advance.
//!
//! - **There is no UDP mux** (`SettingEngine::set_udp_network` is
//!   commented out as `/*todo:*/`), so the libp2p WebRTC-Direct trick of
//!   reading the peer's ufrag off the first STUN packet before building
//!   a peer connection is not available without upstream work.
//!
//! # So the shape that does work
//!
//! Each side publishes once and neither waits for a reply: this process
//! publishes `{ host, port, certhash, ufrag, pwd }`, and a dialer
//! publishes its own `{ ufrag, pwd }` as it starts dialing. ICE
//! retransmits, so checks are still arriving while the far side picks
//! the record up — but Chromium abandons failed checks after roughly
//! thirty seconds, so that is the budget the delivery path has.
//!
//! `--peer-ufrag` and `--peer-candidate` exist to re-measure which parts
//! are load-bearing: supply the real value for one and leave the rest
//! wrong.
//!
//! Run: `cargo run -p tonk-rtc --example direct_dial -- --bind 127.0.0.1:0`

use std::sync::Arc;

use webrtc::data_channel::{DataChannel, DataChannelEvent};
use webrtc::peer_connection::{
    PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCConfigurationBuilder,
    RTCPeerConnectionState, RTCSessionDescription, SettingEngine,
};

/// The ICE credentials this process publishes. Static so a peer can
/// learn them from an address record instead of a handshake.
const UFRAG: &str = "tonkdial";
const PWD: &str = "tonkdialtonkdialtonkdialtonk";

/// What the fabricated offer claims about the peer, when the caller
/// supplies nothing better. Wrong on purpose: the point is to find out
/// whether being wrong matters, and which parts.
const PEER_UFRAG: &str = "PLACEHOLD";
const PEER_PWD: &str = "PLACEHOLDPLACEHOLDPLACEHOLD1";

fn fabricated_offer(peer_ufrag: &str, peer_pwd: &str, peer_candidate: Option<&str>) -> String {
    let fingerprint = ["AB"; 32].join(":");
    // A lite agent "only provides host candidates" and may not form a
    // peer-reflexive candidate for a source it was never told about.
    // Supplying the peer's real address tests exactly that.
    let candidate = match peer_candidate {
        Some(address) => {
            let (host, port) = address.split_once(' ').unwrap_or((address, "0"));
            format!(
                "a=candidate:1 1 udp 2130706431 {host} {port} typ host\r\na=end-of-candidates\r\n"
            )
        }
        None => String::new(),
    };
    format!(
        "v=0\r\n\
         o=- 0 0 IN IP4 0.0.0.0\r\n\
         s=-\r\n\
         t=0 0\r\n\
         a=fingerprint:sha-256 {fingerprint}\r\n\
         a=group:BUNDLE 0\r\n\
         m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n\
         c=IN IP4 0.0.0.0\r\n\
         a=setup:actpass\r\n\
         a=mid:0\r\n\
         a=sendrecv\r\n\
         a=sctp-port:5000\r\n\
         a=max-message-size:65536\r\n\
         a=ice-ufrag:{peer_ufrag}\r\n\
         a=ice-pwd:{peer_pwd}\r\n{candidate}"
    )
}

struct Watcher;

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Watcher {
    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        eprintln!("[spike] connection state: {state:?}");
    }

    async fn on_data_channel(&self, channel: Arc<dyn DataChannel>) {
        eprintln!("[spike] the browser opened a data channel");
        tokio::spawn(async move {
            while let Some(event) = channel.poll().await {
                match event {
                    DataChannelEvent::OnOpen => {
                        println!("RESULT connected");
                        let _ = channel.send_text("hello from the spike").await;
                    }
                    DataChannelEvent::OnMessage(message) => {
                        println!("RESULT message {}", String::from_utf8_lossy(&message.data));
                    }
                    DataChannelEvent::OnClose => return,
                    _ => {}
                }
            }
        });
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let mut arguments = std::env::args().skip(1);
    let mut bind: String = "127.0.0.1:0".to_owned();
    let mut peer_ufrag: String = PEER_UFRAG.to_owned();
    let mut peer_pwd: String = PEER_PWD.to_owned();
    let mut peer_candidate: Option<String> = None;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--bind" => bind = arguments.next().unwrap_or(bind),
            // Supplying the peer's REAL ufrag (while leaving the
            // password wrong) isolates how much of the peer's half this
            // side actually has to know.
            "--peer-ufrag" => peer_ufrag = arguments.next().unwrap_or(peer_ufrag),
            "--peer-pwd" => peer_pwd = arguments.next().unwrap_or(peer_pwd),
            "--peer-candidate" => peer_candidate = arguments.next(),
            _ => {}
        }
    }

    let mut settings = SettingEngine::default();
    // Never send connectivity checks: that is what removes the need for
    // the browser's ICE password.
    settings.set_lite(true);
    // Publishable, so a peer can build our half of the handshake.
    settings.set_ice_credentials(UFRAG.to_owned(), PWD.to_owned());
    // We will never learn the browser's fingerprint, so we cannot check
    // it. The browser still checks OURS, so this connection is
    // one-way authenticated — which is why a real version needs an
    // application-layer handshake on top.
    settings.disable_certificate_fingerprint_verification(true);

    let connection: Arc<dyn PeerConnection> = Arc::new(
        PeerConnectionBuilder::new()
            .with_configuration(RTCConfigurationBuilder::default().build())
            .with_setting_engine(settings)
            .with_handler(Arc::new(Watcher))
            .with_udp_addrs(vec![bind.clone()])
            .build()
            .await?,
    );

    connection
        .set_remote_description(RTCSessionDescription::offer(fabricated_offer(
            &peer_ufrag,
            &peer_pwd,
            peer_candidate.as_deref(),
        ))?)
        .await?;
    let answer = connection.create_answer(None).await?;
    connection.set_local_description(answer).await?;

    // Host candidates only (lite), so gathering is immediate; a short
    // settle is enough and avoids depending on "complete" arriving.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let local = connection
        .local_description()
        .await
        .ok_or("no local description")?;
    println!("ANSWER {}", serde_json::to_string(&local.sdp)?);

    tokio::time::sleep(std::time::Duration::from_secs(45)).await;
    println!("RESULT timeout");
    Ok(())
}
