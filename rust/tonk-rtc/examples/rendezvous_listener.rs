//! A listener presenting the rendezvous certificate, for the browser
//! end-to-end test to dial.
//!
//! Deliberately smaller than `tonk rtc serve`: no site, no iroh, no
//! dialog. It answers a dial and reports what it accepted, so a failure
//! is unambiguously the handshake rather than anything above it.

use std::io::Write as _;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let port = tonk_rtc::rendezvous::port(tonk_rtc::rendezvous::RENDEZVOUS);
    let identity = tonk_rtc::Identity::rendezvous()?;

    let listener = tonk_rtc::dial::listen(identity, port).await?;

    // The harness waits for this line before driving the browser.
    println!("LISTENING port={port}");
    std::io::stdout().flush()?;

    let dialer = listener
        .accept_datagram()
        .await
        .ok_or_else(|| anyhow::anyhow!("the listener stopped before anyone dialed"))?;

    println!(
        "ACCEPTED ufrag={} label={}",
        dialer.ufrag,
        dialer.channel.label()
    );
    std::io::stdout().flush()?;
    Ok(())
}
