//! `tonk rtc connect` — a WebRTC data channel to a browser tab.
//!
//! A proof of concept, and scoped like one: what crosses the channel is
//! lines of text. The point is the channel, not the payload — the
//! intent is that it eventually carries dialog's remote effects so the
//! CLI can serve as a sync remote for a browser that has no other way
//! to reach it.
//!
//! # Why WebRTC rather than the loopback server the CLI already has
//!
//! Because Safari will not let a page on `https://tonk.network` open a
//! `fetch` or a WebSocket to `http://127.0.0.1` — that is mixed content,
//! and now a Local Network Access prompt besides. A *navigation* to
//! loopback is still allowed, which is why the account-login ceremony
//! works, but a navigation is one shot and kills the page. There is no
//! portable way for a live browser page to hold a connection to a local
//! process except WebRTC.
//!
//! # The ceremony
//!
//! Deliberately the same shape as account login:
//!
//! ```text
//!   tonk                                        browser
//!   ----                                        -------
//!   bind loopback listener
//!   create offer, gather ICE
//!   open <via>#offer=…&callback=…  ──────────>  read fragment, answer
//!                                               open callback#answer=…
//!   receive answer on loopback     <──────────  (popup posts it back)
//!   set remote description
//!   ══════════════ data channel ══════════════
//! ```
//!
//! Both hops are navigations carrying their payload in a URL fragment,
//! which is what keeps them clear of CORS, Local Network Access, and
//! server logs.
//!
//! # The part that does not survive contact with a second machine
//!
//! Loopback signalling only works because the browser and this process
//! share a machine. It is a scaffold, not the design: the intended
//! channel is the replicated space itself — descriptions written as
//! facts, read by whichever peer is listening. That bootstraps over the
//! existing remote and needs no new infrastructure, and it works
//! between peers that have never shared a host.
//!
//! When that lands, the shape to keep is the one this module already
//! depends on: bind a channel, hand out an offer, wait for an answer.
//! Nothing below reaches into how those bytes travel.

use std::io::Write as _;

use anyhow::{Context as _, Result, bail};
use tonk_rtc::{Loopback, peer};

/// Where the browser half lives when nobody says otherwise.
pub const DEFAULT_RTC_PAGE: &str = "https://tonk.network/rtc";

/// How the caller wants the ceremony run.
#[derive(Debug, Clone, Default)]
pub struct ConnectOptions {
    /// The page that answers the offer. Defaults to [`DEFAULT_RTC_PAGE`];
    /// point it at `http://127.0.0.1:8080/rtc` for a `dev:web` server.
    pub via: Option<String>,
    /// Print the URL instead of opening a browser.
    pub no_open: bool,
    /// STUN servers for ICE. Empty is correct for two processes on one
    /// machine — host candidates pair up without help.
    pub stun: Vec<String>,
}

/// Validate the page this process is about to send someone to.
///
/// The offer rides in that URL's fragment, so a mistyped `--via` is a
/// URL handed to a stranger. Reject anything that is not an ordinary
/// http(s) page, and strip credentials and any fragment the caller
/// supplied — the fragment is ours.
fn answering_page(explicit: Option<&str>) -> Result<url::Url> {
    let mut page = match explicit {
        Some(explicit) => url::Url::parse(explicit).context("--via is not a valid URL")?,
        None => url::Url::parse(DEFAULT_RTC_PAGE).expect("the built-in page is a valid URL"),
    };
    if !matches!(page.scheme(), "http" | "https") || page.host_str().is_none() {
        bail!("the answering page must be an http or https URL");
    }
    let _ = page.set_username("");
    let _ = page.set_password(None);
    page.set_fragment(None);
    Ok(page)
}

/// How the caller wants the listener run.
#[derive(Debug, Clone, Default)]
pub struct ListenOptions {
    /// The page that dials. Defaults to Tonk's own `/rtc`; point it at
    /// a `dev:web` server for local work.
    pub via: Option<String>,
    /// Print the URL instead of opening a browser.
    pub no_open: bool,
    /// The UDP port to listen on. Defaults to
    /// [`tonk_rtc::dial::DEFAULT_PORT`], which is what makes the
    /// address predictable enough for a dialer to assume it.
    pub port: Option<u16>,
}

/// Where this machine's WebRTC certificate lives.
///
/// Beside the other local state the CLI keeps. It contains a private
/// key, so it is written with the same care as the rest.
fn identity_path() -> Result<std::path::PathBuf> {
    let data = dirs::data_dir().context("could not determine platform data directory")?;
    Ok(data.join("tonk").join("rtc-identity.pem"))
}

/// Load this machine's WebRTC certificate, minting one the first time.
///
/// Persisted because a published address names its fingerprint: mint a
/// fresh one per run and every address handed out before a restart
/// stops authenticating this side.
fn rtc_identity() -> Result<tonk_rtc::Identity> {
    let path = identity_path()?;
    if let Ok(pem) = std::fs::read_to_string(&path)
        && let Ok(identity) = tonk_rtc::Identity::from_pem(&pem)
    {
        return Ok(identity);
    }

    let identity = tonk_rtc::Identity::generate().context("could not mint a WebRTC certificate")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    // A failure to persist is not a failure to listen — this run still
    // works, its address just will not outlive the process.
    if let Err(error) = write_private(&path, &identity.to_pem()) {
        eprintln!(
            "warning: could not save the WebRTC certificate ({error}); this listener's address will not survive a restart"
        );
    }
    Ok(identity)
}

/// Write key material readable only by its owner.
fn write_private(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(contents.as_bytes())
    }
    #[cfg(not(unix))]
    std::fs::write(path, contents)
}

/// Publish an address and wait to be dialed.
///
/// The opposite direction from [`connect`], and the point of it is what
/// does NOT happen: nothing travels from the browser back to here. The
/// address carries everything a dialer needs — candidates, DTLS
/// fingerprint, and a shared ICE credential — so the handshake is one
/// way and involves no signalling channel at all.
///
/// For the proof of concept the address still reaches the browser
/// through a URL, because that is the shortest path to a demo. That URL
/// is a stand-in for a cardinality-one fact in a replicated space: a
/// peer reads the record whenever sync delivers it and dials later,
/// with no coordination. Discovery tolerates arbitrary latency; the
/// handshake involves no sync at all.
pub async fn listen(options: ListenOptions) -> Result<()> {
    let page = answering_page(options.via.as_deref())?;

    let port = options.port.unwrap_or(tonk_rtc::dial::DEFAULT_PORT);
    let listener = tonk_rtc::dial::listen(rtc_identity()?, port)
        .await
        .context("could not start the WebRTC listener")?;
    let address = listener.address();

    let target = format!("{page}#address={}", address.encode());
    println!(
        "listening on {}",
        address
            .candidates
            .iter()
            .map(|candidate| format!("{}:{}", candidate.host, candidate.port))
            .collect::<Vec<_>>()
            .join(", ")
    );

    if options.no_open {
        println!("open this in your browser:\n\n{target}\n");
    } else {
        println!("opening {page} …");
        if webbrowser::open(&target).is_err() {
            println!("could not open a browser. open this yourself:\n\n{target}\n");
        }
    }
    println!("waiting to be dialed…");

    let session = listener
        .accept()
        .await
        .ok_or_else(|| anyhow::anyhow!("the listener stopped before anyone dialed"))?;

    // The listener keeps serving dials — the address is reusable, and
    // several tabs may hold channels at once. This relays the first one
    // because the proof of concept has a single terminal to relay to;
    // carrying more than one is the dispatcher's job, not this one's.
    println!("connected. type a line to send it to the browser; ctrl-d to hang up.\n");
    relay(session).await
}

/// Run the ceremony and then relay lines until one side hangs up.
pub async fn connect(options: ConnectOptions) -> Result<()> {
    let page = answering_page(options.via.as_deref())?;

    let signalling = Loopback::bind()
        .await
        .context("could not start the local signalling listener")?;

    let offering = peer::offer(options.stun.clone())
        .await
        .context("could not create the WebRTC offer")?;

    let target = format!(
        "{page}#offer={}&callback={}",
        offering.offer(),
        urlencoding::encode(signalling.url())
    );

    if options.no_open {
        println!("open this in your browser:\n\n{target}\n");
    } else {
        println!("opening {page} …");
        if webbrowser::open(&target).is_err() {
            println!("could not open a browser. open this yourself:\n\n{target}\n");
        }
    }
    println!("waiting for the browser to answer…");

    let answer = signalling
        .receive()
        .await
        .context("the browser did not answer")?;

    let session = offering
        .accept(&answer)
        .await
        .context("the answer did not produce an open data channel")?;

    println!("connected. type a line to send it to the browser; ctrl-d to hang up.\n");

    relay(session).await
}

/// Pump stdin to the peer and the peer to stdout until either ends.
///
/// stdin is read on a blocking thread because there is no portable
/// async stdin: `spawn_blocking` keeps the reactor free to service the
/// connection while a read is parked.
async fn relay(session: peer::Session) -> Result<()> {
    let (lines, mut typed) = tokio::sync::mpsc::unbounded_channel();
    tokio::task::spawn_blocking(move || {
        for line in std::io::stdin().lines() {
            match line {
                Ok(line) => {
                    if lines.send(line).is_err() {
                        return;
                    }
                }
                Err(_) => return,
            }
        }
    });

    loop {
        tokio::select! {
            line = typed.recv() => match line {
                // End of stdin: the user is done talking, so close
                // rather than sit on a half-useful channel.
                None => break,
                Some(line) if line.trim().is_empty() => continue,
                Some(line) => {
                    if let Err(error) = session.send(&line).await {
                        eprintln!("could not send: {error}");
                        break;
                    }
                }
            },
            message = session.recv() => match message {
                None => {
                    println!("the browser closed the channel.");
                    break;
                }
                Some(message) => {
                    println!("browser: {message}");
                    let _ = std::io::stdout().flush();
                }
            },
        }
    }

    session.close().await.ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_page_is_used_when_none_is_given() {
        assert_eq!(answering_page(None).unwrap().as_str(), DEFAULT_RTC_PAGE);
    }

    #[test]
    fn a_dev_server_page_is_accepted() {
        assert_eq!(
            answering_page(Some("http://127.0.0.1:8080/rtc"))
                .unwrap()
                .as_str(),
            "http://127.0.0.1:8080/rtc"
        );
    }

    /// The offer rides in this URL's fragment, so a caller-supplied
    /// fragment must not survive to collide with it.
    #[test]
    fn a_supplied_fragment_is_dropped() {
        let page = answering_page(Some("https://example.test/rtc#offer=theirs")).unwrap();
        assert_eq!(page.fragment(), None);
        assert_eq!(page.as_str(), "https://example.test/rtc");
    }

    #[test]
    fn credentials_are_stripped_from_the_page() {
        let page = answering_page(Some("https://user:secret@example.test/rtc")).unwrap();
        assert_eq!(page.username(), "");
        assert_eq!(page.password(), None);
        assert!(!page.as_str().contains("secret"));
    }

    #[test]
    fn a_non_http_scheme_is_refused() {
        for bad in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,<script>",
            "not a url",
        ] {
            assert!(answering_page(Some(bad)).is_err(), "accepted {bad}");
        }
    }
}
