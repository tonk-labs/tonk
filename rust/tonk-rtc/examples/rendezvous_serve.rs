//! A rendezvous listener that answers dialog invocations.
//!
//! What `tonk rtc serve` does, without a site: the store is
//! `dialog-iroh-remote`'s volatile helper rather than an operator, so a
//! failure is the transport or the protocol and never a missing space.
//! Prints its `did:key` so the harness can dial it.

use std::io::Write as _;
use std::sync::Arc;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let phrase = tonk_rtc::rendezvous::RENDEZVOUS;
    let port = tonk_rtc::rendezvous::port(phrase);

    let listener = tonk_rtc::dial::listen(tonk_rtc::Identity::rendezvous()?, port).await?;
    let transport = tonk_rtc::transport::WebRtcTransport::new(tonk_rtc::rendezvous::transport_tag(
        phrase,
        tonk_rtc::rendezvous::Side::Listener,
    ));

    let key = iroh::SecretKey::generate();
    let peer = dialog_iroh_remote::site::IrohAddress::from(iroh::EndpointAddr::from(key.public()));
    let endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::Empty)
        .crypto_provider(iroh::tls::default_provider())
        .secret_key(key)
        .alpns(vec![dialog_iroh_remote::transport::ALPN.to_vec()])
        .add_custom_transport(transport.clone())
        .bind()
        .await?;

    println!("PEER {}", peer.did());
    println!("LISTENING port={port}");
    std::io::stdout().flush()?;

    let pumping = {
        let transport = transport.clone();
        tokio::spawn(async move {
            while let Some(dialer) = listener.accept_datagram().await {
                println!("ACCEPTED ufrag={}", dialer.ufrag);
                let _ = std::io::stdout().flush();
                tonk_rtc::transport::attach(
                    &transport,
                    tonk_rtc::transport::rendezvous_addr(
                        phrase,
                        tonk_rtc::rendezvous::Side::Dialer,
                    ),
                    dialer.channel,
                );
            }
        })
    };

    let responder = Arc::new(dialog_iroh_remote::serve::Responder::new(
        dialog_iroh_remote::helpers::Volatile::default(),
        dialog_did_web::CachingResolver::new(dialog_did_web::WebResolver::new()),
    ));
    dialog_iroh_remote::transport::accept(endpoint, responder).await;

    pumping.abort();
    Ok(())
}
