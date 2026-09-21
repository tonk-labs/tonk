//! WebRTC carriers between the CLI and a browser tab.
//!
//! `serve` exposes the profile's registered spaces through signed Dialog
//! effects over iroh/QUIC datagrams. Connecting discloses the mounted names
//! and IDs, not authority to read or write their contents. The listener is
//! loopback-only. `connect` and `listen` retain the text-channel diagnostics.
//!
//! # Why WebRTC rather than the loopback server the CLI already has
//!
//! WebRTC provides a datagram path without making the page use loopback
//! HTTP fetches or WebSockets. Those face mixed-content and local-network
//! policies that differ by browser. A navigation can carry a one-shot
//! account-login response but cannot carry a live repository stream.
//! WebRTC still has browser-specific ICE and permission constraints; it is
//! not a promise to bypass the browser's network policy.
//!
//! # The text diagnostic's offer/answer ceremony
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
    // An explicitly isolated installation must never share the default
    // installation's certificate, including in product tests.
    match std::env::var("TONK_PROFILE_DIRECTORY") {
        Ok(path) if !path.is_empty() => {
            return Ok(std::path::PathBuf::from(path).join("rtc-identity.pem"));
        }
        Ok(_) => bail!("TONK_PROFILE_DIRECTORY must not be empty"),
        Err(std::env::VarError::NotPresent) => {}
        Err(error) => return Err(error).context("invalid TONK_PROFILE_DIRECTORY"),
    }
    let data = dirs::data_dir().context("could not determine platform data directory")?;
    Ok(data.join("tonk").join("rtc-identity.pem"))
}

/// The certificate this listener presents.
///
/// Derived from the rendezvous phrase, so a browser needs nothing
/// published: it derives the same fingerprint, the port comes from the
/// same phrase, and the candidate is loopback — which is the whole
/// "nothing is exchanged" property.
///
/// A per-machine identity is still minted and persisted when
/// `TONK_RTC_PRIVATE_IDENTITY` is set, for anyone who would rather hand
/// out an address than share a certificate. It costs a dialer the
/// address it would otherwise not have needed.
fn rtc_identity() -> Result<tonk_rtc::Identity> {
    if std::env::var_os("TONK_RTC_PRIVATE_IDENTITY").is_none() {
        return tonk_rtc::Identity::rendezvous()
            .context("the rendezvous certificate would not derive");
    }
    private_identity(&identity_path()?)
}

/// A saved private route must keep its certificate as well as its endpoint
/// key. Serialize first use and never turn corrupt/unreadable state into a
/// different fingerprint. This lock protects only short local filesystem work.
fn private_identity(path: &std::path::Path) -> Result<tonk_rtc::Identity> {
    let parent = path.parent().context("certificate path has no parent")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("could not create {}", parent.display()))?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_extension("pem.lock"))
        .context("could not open the WebRTC certificate lock")?;
    lock.lock()
        .context("could not lock the WebRTC certificate")?;
    match std::fs::read_to_string(path) {
        Ok(pem) => {
            return tonk_rtc::Identity::from_pem(&pem)
                .context("stored WebRTC certificate is corrupt; refusing to replace it");
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .context("could not read the WebRTC certificate; refusing to replace it");
        }
    }
    let identity = tonk_rtc::Identity::generate().context("could not mint a WebRTC certificate")?;
    write_private(path, &identity.to_pem())
        .context("could not persist the WebRTC certificate; refusing an ephemeral private route")?;
    Ok(identity)
}

/// Atomically create owner-only key material; never truncate an existing file.
fn write_private(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("no certificate parent"))?;
    // NamedTempFile creates the file with owner-only permissions on Unix.
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(contents.as_bytes())?;
    file.flush()?;
    file.as_file().sync_all()?;
    file.persist_noclobber(path).map_err(|error| error.error)?;
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
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

    let port = options
        .port
        .unwrap_or_else(|| tonk_rtc::rendezvous::port(tonk_rtc::rendezvous::RENDEZVOUS));
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
    fn concurrent_private_certificate_creation_and_reopen_keep_one_fingerprint() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("rtc-identity.pem");
        let barrier = std::sync::Barrier::new(4);
        let fingerprints = std::thread::scope(|scope| {
            let tasks: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        private_identity(&path).unwrap().fingerprint()
                    })
                })
                .collect();
            tasks
                .into_iter()
                .map(|task| task.join().unwrap())
                .collect::<Vec<_>>()
        });
        let fingerprint = private_identity(&path)?.fingerprint();
        assert!(fingerprints.iter().all(|found| found == &fingerprint));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path)?.permissions().mode() & 0o077, 0);
        }
        Ok(())
    }

    #[test]
    fn corrupt_and_unreadable_private_certificates_are_not_replaced() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("rtc-identity.pem");
        std::fs::write(&path, "invalid certificate")?;
        assert!(
            private_identity(&path)
                .unwrap_err()
                .to_string()
                .contains("corrupt")
        );
        assert_eq!(std::fs::read_to_string(&path)?, "invalid certificate");
        let unreadable = directory.path().join("is-a-directory.pem");
        std::fs::create_dir(&unreadable)?;
        assert!(
            private_identity(&unreadable)
                .unwrap_err()
                .to_string()
                .contains("could not read")
        );
        assert!(unreadable.is_dir());
        Ok(())
    }

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

#[cfg(feature = "rtc")]
mod serving {
    use super::*;

    /// Profile-wide serving state, deliberately without a selected repository.
    /// The operator's base is installation state; only explicit registry mounts
    /// are offered to peers. Opening this context never creates a user space.
    struct RegistryContext {
        profile: dialog_operator::Profile,
        operator: dialog_operator::Operator<dialog_storage::provider::storage::NativeSpace>,
        storage: dialog_storage::provider::storage::Storage<
            dialog_storage::provider::storage::NativeSpace,
        >,
        account_store: crate::space::SpaceStore,
    }

    impl RegistryContext {
        async fn open(config: crate::site::SiteConfig) -> Result<Self> {
            std::fs::create_dir_all(config.account_store.root())
                .context("could not open serving profile state")?;
            let (profile, operator, storage) =
                crate::site::build_profile_and_operator(config.account_store.root(), &config)
                    .await?;
            Ok(Self {
                profile,
                operator,
                storage,
                account_store: config.account_store,
            })
        }
    }

    #[cfg(test)]
    impl From<&crate::site::TonkSite> for RegistryContext {
        fn from(site: &crate::site::TonkSite) -> Self {
            Self {
                profile: site.profile.clone(),
                operator: site.operator.inner().clone(),
                storage: site.storage.clone(),
                account_store: site.account_store.clone(),
            }
        }
    }

    /// Every space this machine holds, served as one peer.
    ///
    /// `tonk rtc serve` used to serve the *selected* space: one
    /// operator, one mounted repository, and an invocation naming
    /// anything else answered `SubjectNotFound`. A peer is the machine
    /// though, not whichever space a shell happened to be pointed at,
    /// so this mounts the registry and serves all of it.
    ///
    /// Mounting is eager, at startup, rather than on first invocation.
    /// A lazy mount belongs in the router's miss path, which is where
    /// `SubjectNotFound` is decided and is several layers below anything
    /// that knows what a registry is. Eager costs one credential read
    /// per registered space and makes the answer to "what do you hold"
    /// true by construction: the peer offers exactly what it mounted, so
    /// nothing it lists can fail to route.
    ///
    /// The names come from the registry because nothing else has them.
    /// A `Storage` pool is keyed by DID and its own `peer::Spaces`
    /// answer is nameless for that reason; `spaces.json` is where a
    /// space is called `notes`, and a browser listing spaces it has
    /// never opened has nowhere else to read a label from.
    pub struct Served {
        /// Everything but the inventory. The operator routes each
        /// invocation to the space its subject names, over the storage
        /// every mount landed in.
        operator: dialog_operator::Operator<dialog_storage::provider::storage::NativeSpace>,
        gate: std::sync::Arc<AccountGate>,

        /// What the registry calls each space that mounted, in the order
        /// the registry lists them.
        offers: Vec<dialog_effects::peer::Offer>,
    }

    struct AccountGate {
        operator: dialog_operator::Operator<dialog_storage::provider::storage::NativeSpace>,
        profile: dialog_operator::Profile,
        store: crate::space::SpaceStore,
        account: Option<crate::account_session::ActiveAccount>,
    }

    /// Mount every registered space into `site`'s environment.
    ///
    /// A space that will not mount is reported and skipped rather than
    /// failing the serve: one unreadable directory should not take the
    /// other spaces offline, and a peer that silently omitted it would
    /// leave the operator staring at a space the CLI still lists.
    async fn mount_registry(site: &RegistryContext) -> Result<Served> {
        let account =
            crate::account_session::snapshot(&site.profile, &site.operator, &site.account_store)
                .await?
                .active;
        let registry = site
            .account_store
            .load()
            .context("could not read the space registry")?;

        let mut offers = Vec::with_capacity(registry.spaces.len());
        for (name, entry) in &registry.spaces {
            match crate::site::mount_space(&site.storage, &entry.site).await {
                Ok(subject) => offers.push(dialog_effects::peer::Offer {
                    subject,
                    name: Some(name.clone()),
                }),
                Err(error) => {
                    eprintln!("warning: not serving '{name}': {error:#}");
                }
            }
        }

        Ok(Served {
            operator: site.operator.clone(),
            gate: std::sync::Arc::new(AccountGate {
                operator: site.operator.clone(),
                profile: site.profile.clone(),
                store: site.account_store.clone(),
                account,
            }),
            offers,
        })
    }

    impl AccountGate {
        /// A server is bound to the account attachment it mounted. Hold the
        /// same cross-process guard used by normal remote dispatch until the
        /// operation finishes; a replacement must restart with fresh mounts.
        async fn guard(
            &self,
        ) -> Result<
            crate::account_session::AccountSessionReadGuard,
            dialog_capability::access::AuthorizeError,
        > {
            use dialog_capability::access::AuthorizeError;
            let unavailable = |error: anyhow::Error| AuthorizeError::Unavailable {
                detail: error.to_string(),
            };
            let guard =
                crate::account_session::shared_remote_guard(&self.store).map_err(unavailable)?;
            let active =
                crate::account_session::active_guarded(&self.profile, &self.operator, &guard)
                    .await
                    .map_err(unavailable)?;
            if active != self.account {
                return Err(AuthorizeError::Unavailable {
                    detail: "the serving account changed; restart `tonk rtc serve`".into(),
                });
            }
            Ok(guard)
        }
    }

    impl Served {
        /// What this peer will offer, for the caller that prints it.
        pub fn offers(&self) -> &[dialog_effects::peer::Offer] {
            &self.offers
        }
    }

    macro_rules! serve_guarded {
        ($($command:ty),+ $(,)?) => {$(
            #[async_trait::async_trait]
            impl dialog_capability::Provider<$command> for Served {
                async fn execute(
                    &self,
                    input: <$command as dialog_capability::Command>::Input,
                ) -> <$command as dialog_capability::Command>::Output {
                    let _guard = self.gate.guard().await?;
                    dialog_capability::Provider::<$command>::execute(&self.operator, input).await
                }
            }
        )+};
    }
    serve_guarded!(
        dialog_effects::archive::Get,
        dialog_effects::archive::Put,
        dialog_effects::archive::Import,
        dialog_effects::memory::Resolve,
        dialog_effects::memory::Publish,
        dialog_effects::memory::Retract,
        dialog_effects::peer::Hello,
    );

    // Streams outlive the dispatch that creates them. Check each chunk and
    // the final commit under the same account lock, without retaining that
    // lock while waiting for an untrusted peer to supply the next chunk.
    struct AccountReader {
        inner: dialog_effects::blob::BlobReader,
        gate: std::sync::Arc<AccountGate>,
    }
    #[async_trait::async_trait]
    impl dialog_effects::blob::BlobSource for AccountReader {
        async fn next(&mut self) -> Result<Option<Vec<u8>>, dialog_effects::blob::BlobError> {
            let _guard = self.gate.guard().await?;
            self.inner.next().await
        }
    }
    struct AccountWriter {
        inner: dialog_effects::blob::BlobWriter,
        gate: std::sync::Arc<AccountGate>,
    }
    #[async_trait::async_trait]
    impl dialog_effects::blob::BlobSink for AccountWriter {
        async fn write_all(&mut self, bytes: &[u8]) -> Result<(), dialog_effects::blob::BlobError> {
            let _guard = self.gate.guard().await?;
            self.inner.write_all(bytes).await
        }
        async fn finish(
            self: Box<Self>,
        ) -> Result<dialog_common::Blake3Hash, dialog_effects::blob::BlobError> {
            let _guard = self.gate.guard().await?;
            self.inner.finish().await
        }
    }
    macro_rules! serve_stream {
        ($command:ty, $wrapper:ident) => {
            #[async_trait::async_trait]
            impl dialog_capability::Provider<$command> for Served {
                async fn execute(
                    &self,
                    input: <$command as dialog_capability::Command>::Input,
                ) -> <$command as dialog_capability::Command>::Output {
                    let _guard = self.gate.guard().await?;
                    let inner =
                        dialog_capability::Provider::<$command>::execute(&self.operator, input)
                            .await?;
                    Ok(Box::new($wrapper {
                        inner,
                        gate: self.gate.clone(),
                    }))
                }
            }
        };
    }
    serve_stream!(dialog_effects::blob::Read, AccountReader);
    serve_stream!(dialog_effects::blob::Write, AccountWriter);
    serve_stream!(dialog_effects::blob::Import, AccountWriter);

    /// The registry, as the peer offers it.
    #[async_trait::async_trait]
    impl dialog_capability::Provider<dialog_effects::peer::Spaces> for Served {
        async fn execute(
            &self,
            _input: dialog_capability::Capability<dialog_effects::peer::Spaces>,
        ) -> Result<Vec<dialog_effects::peer::Offer>, dialog_effects::peer::PeerError> {
            let _guard = self.gate.guard().await?;
            Ok(self.offers.clone())
        }
    }

    /// The credential-store key this profile's peer seed is held under.
    ///
    /// The same name the worker uses, because it is the same identity:
    /// a device profile has one peer key whether a browser or this
    /// binary binds the endpoint.
    const PEER_SEED_SITE: &str = "tonk:peer-seed";

    /// This device profile's iroh secret key, minting one the first time.
    ///
    /// Held in the profile's credential store rather than a file beside
    /// the binary. The endpoint key *is* the peer's name, so it has to
    /// survive a restart — a remote that recorded this peer must keep
    /// reaching it — and it has to be per device profile, which is what
    /// the profile-scoped store already is.
    ///
    /// Replaces the old `rtc-endpoint.key`: that file was a third
    /// identity with its own lifecycle, which is what made a peer
    /// something you had to copy a `did:key` for rather than something
    /// a profile simply *is*.
    async fn endpoint_key(site: &RegistryContext) -> Result<iroh::SecretKey> {
        use dialog_capability::SiteId;
        use dialog_effects::credential::Secret;

        // Serialize first use across concurrent CLI processes in the same
        // installation. Hold the lock through credential persistence, not
        // just random generation. A dropped process releases the OS lock.
        let lock_path = site.account_store.root().join("rtc-peer-key.lock");
        let _lock = tokio::task::spawn_blocking(move || -> Result<std::fs::File> {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&lock_path)
                .context("could not open peer identity lock")?;
            file.lock().context("could not lock peer identity")?;
            Ok(file)
        })
        .await
        .context("peer identity lock task failed")??;

        let handle = || {
            site.profile
                .credential()
                .site(SiteId::from(PEER_SEED_SITE.to_owned()))
        };

        match handle().load::<Secret>().perform(&site.operator).await {
            Ok(secret) => {
                let seed = <[u8; 32]>::try_from(secret.as_bytes())
                    .context("stored peer identity is corrupt; refusing to replace it")?;
                return Ok(iroh::SecretKey::from_bytes(&seed));
            }
            Err(error) if crate::account_state::credential_is_missing(&error) => {}
            Err(error) => {
                return Err(error).context("could not read peer identity; refusing to replace it");
            }
        }

        // iroh's own generator rather than a fresh entropy dependency:
        // the bytes it mints are exactly the 32 a secret key is.
        let key = iroh::SecretKey::generate();
        let seed = key.to_bytes();
        handle()
            .save(Secret::from(seed.to_vec()))
            .perform(&site.operator)
            .await
            .context("could not store the peer seed")?;
        Ok(key)
    }

    /// Serve this site's spaces to peers that dial in.
    ///
    /// The address printed is the whole point: a peer's `did:key`, stable
    /// across restarts because the key is persisted, and a `?route=` hint
    /// holding the local-dial record. Paste the complete URI into the
    /// browser's network page: it pins both the identity and exact route.
    pub async fn serve(config: crate::site::SiteConfig, options: ListenOptions) -> Result<()> {
        let site = RegistryContext::open(config).await?;
        // Mount failures are reported before binding. An empty registry is a
        // useful discovery result, not an excuse to create an arbitrary space.
        let served = mount_registry(&site).await?;
        let revocations = crate::revocations::Cached::for_registry(
            &site.profile,
            &site.operator,
            &site.account_store,
            served.offers(),
        )
        .await?;
        let resolver =
            crate::resolution::Cached::new(site.account_store.root().join("did-evidence"))?;
        if !revocations.has_sources() {
            eprintln!(
                "warning: no configured revocation service; delegated content requests will be refused until one is configured and the server restarted"
            );
        }

        // A span, not a port: several `tonk`s on one machine each take
        // their own slot and stay findable, because a dialer that knows
        // the phrase knows the whole range. `--port` still pins one,
        // which is what a test or a second machine wants.
        let listener = match options.port {
            Some(port) => tonk_rtc::dial::listen(rtc_identity()?, port).await,
            None => {
                tonk_rtc::dial::listen_in(
                    rtc_identity()?,
                    tonk_rtc::rendezvous::ports(tonk_rtc::rendezvous::RENDEZVOUS),
                )
                .await
            }
        }
        .context("could not start the WebRTC listener")?;
        let port = listener.address().candidates[0].port;

        // The transport announces itself *as* the local-dial address, which
        // is what lets one `?route=` hint carry everything a dialer needs:
        // candidates and the DTLS fingerprint, inside iroh's opaque custom
        // address.
        let transport = tonk_rtc::transport::WebRtcTransport::new(listener.address().encode());
        let route = transport.local_addr();

        let key = endpoint_key(&site).await?;
        let id = key.public();
        let endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::Empty)
            .crypto_provider(iroh::tls::default_provider())
            .secret_key(key)
            // dialog's ALPN, not this crate's: a peer must be speaking the
            // remote protocol, not merely be reachable.
            .alpns(vec![dialog_iroh_remote::transport::ALPN.to_vec()])
            .add_custom_transport(transport.clone())
            .bind()
            .await
            .context("could not bind the iroh endpoint")?;

        let peer = dialog_iroh_remote::site::IrohAddress::from(iroh::EndpointAddr {
            id,
            addrs: [iroh::TransportAddr::Custom(route)].into_iter().collect(),
        });

        match served.offers() {
            [] => println!("no spaces to serve; `tonk space new <name>` makes one.\n"),
            offers => {
                println!("serving {} space(s) to peers that dial in:\n", offers.len());
                for offer in offers {
                    println!(
                        "  {}  {}",
                        offer.name.as_deref().unwrap_or("-"),
                        offer.subject
                    );
                }
                println!();
            }
        }
        println!("  tonk remote add <name> '{}'\n", peer.to_uri());
        println!("PEER {}", peer.to_uri());
        println!(
            "local discovery reveals the listed names and IDs; content still requires authority."
        );
        println!("registry snapshot: restart to include newly registered spaces.");
        println!("listening on port {port}; ctrl-c to stop.\n");

        // Every accepted datagram channel becomes a route iroh can answer
        // on. The ufrag names the dialer: a browser has no address of its
        // own, and this is the value the mux already routed on.
        let pumping = {
            let transport = transport.clone();
            tokio::spawn(async move {
                while let Some(dialer) = listener.accept_datagram().await {
                    let peer = iroh_base::CustomAddr::from_parts(
                        tonk_rtc::transport::TRANSPORT_ID,
                        dialer.ufrag.as_bytes(),
                    );
                    tonk_rtc::transport::attach(&transport, peer, dialer.channel);
                }
            })
        };

        // The registry, not the selected space: a peer is this machine,
        // and an invocation naming any space it holds has to route.
        let responder = std::sync::Arc::new(
            dialog_iroh_remote::serve::Responder::new(served, resolver)
                .with_revocations(revocations),
        );
        let serving_endpoint = endpoint.clone();
        tokio::select! {
            _ = dialog_iroh_remote::transport::accept(serving_endpoint, responder) => {}
            result = tokio::signal::ctrl_c() => { result.context("could not wait for shutdown")?; }
        }

        pumping.abort();
        let _ = pumping.await;
        endpoint.close().await;
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn config(root: &std::path::Path) -> crate::site::SiteConfig {
            crate::site::SiteConfig {
                profile_name: "rtc-identity-test".into(),
                profile_directory: dialog_effects::storage::Directory::At(
                    root.join("profile").to_string_lossy().into_owned(),
                ),
                require_account: false,
                provision_account_spaces: false,
                account_store: crate::space::SpaceStore::at(root.join("state")),
            }
        }

        async fn fixture() -> Result<(tempfile::TempDir, crate::site::TonkSite)> {
            let tmp = tempfile::tempdir()?;
            let config = config(tmp.path());
            std::fs::create_dir_all(config.account_store.root())?;
            let site = crate::site::TonkSite::init_with(tmp.path(), config).await?;
            Ok((tmp, site))
        }

        #[tokio::test]
        async fn empty_registry_needs_no_selected_space_or_account() -> Result<()> {
            let tmp = tempfile::tempdir()?;
            let mut config = config(tmp.path());
            config.require_account = true;
            let context = RegistryContext::open(config.clone()).await?;
            assert!(mount_registry(&context).await?.offers().is_empty());
            let key = endpoint_key(&context).await?.public();
            drop(context);
            let reopened = RegistryContext::open(config.clone()).await?;
            assert_eq!(key, endpoint_key(&reopened).await?.public());
            assert!(mount_registry(&reopened).await?.offers().is_empty());
            assert!(config.account_store.load()?.spaces.is_empty());
            assert!(!crate::site::has_site_data(config.account_store.root()));
            Ok(())
        }

        #[tokio::test]
        async fn registry_mounts_are_a_snapshot_and_bad_entries_do_not_hide_good_ones() -> Result<()>
        {
            let (tmp, site) = fixture().await?;
            let config = config(tmp.path());
            let context = RegistryContext::open(config.clone()).await?;
            let before = mount_registry(&context).await?;
            let mut registry = config.account_store.load()?;
            registry
                .spaces
                .insert("good".into(), crate::space::SpaceEntry::at(&site.root));
            registry.spaces.insert(
                "missing".into(),
                crate::space::SpaceEntry::at(tmp.path().join("missing")),
            );
            config.account_store.save(&registry)?;
            assert!(
                before.offers().is_empty(),
                "existing server must not change its registry snapshot"
            );
            let after = mount_registry(&context).await?;
            assert_eq!(after.offers().len(), 1);
            assert_eq!(after.offers()[0].subject, site.repository.did());
            assert_eq!(after.offers()[0].name.as_deref(), Some("good"));
            // Use the new profile-wide operator, not the selected site's
            // already-mounted environment, to prove the registry mounted it.
            use dialog_capability::Subject;
            use dialog_repository::RepositoryMemoryExt;
            Subject::from(site.repository.did())
                .branch("main")
                .open()
                .perform(&context.operator)
                .await?;
            Ok(())
        }

        #[tokio::test]
        async fn account_transition_stops_existing_server_and_open_blob_commit() -> Result<()> {
            use dialog_capability::Subject;
            use dialog_effects::{Use, archive, blob, peer};
            let (_tmp, site) = fixture().await?;
            let mut account = crate::account_session::ActiveAccount {
                credential_id: "test".into(),
                root_did: site.profile.did().to_string(),
                delegation_cid: "test-state-only".into(),
                delegation_hex: "00".into(),
                remote: None,
                attachment_id: "first".into(),
                attached_at: 1,
            };
            let staged = crate::account_session::stage_activation(
                &site.profile,
                site.operator.inner(),
                &site.account_store,
                account.clone(),
            )
            .await?;
            crate::account_session::finalize_activation(
                &site.profile,
                site.operator.inner(),
                staged,
                &account,
            )
            .await?;
            let context = RegistryContext::from(&site);
            let served = mount_registry(&context).await?;
            let ask = Subject::from(site.profile.did())
                .attenuate(Use)
                .attenuate(peer::Peer)
                .attenuate(peer::Spaces);
            ask.clone().perform(&served).await?;
            let mut writer = Subject::from(site.profile.did())
                .attenuate(Use)
                .attenuate(archive::Archive)
                .attenuate(blob::Blob)
                .invoke(blob::Write)
                .perform(&served)
                .await?;
            writer.write_all(b"must not commit after logout").await?;
            crate::account_session::logout_transition_for_store(
                &site.profile,
                site.operator.inner(),
                &site.account_store,
            )
            .await?;
            assert!(ask.clone().perform(&served).await.is_err());
            assert!(
                writer.finish().await.is_err(),
                "an already-open stream must not commit after logout"
            );
            account.attachment_id = "replacement".into();
            let staged = crate::account_session::stage_activation(
                &site.profile,
                site.operator.inner(),
                &site.account_store,
                account.clone(),
            )
            .await?;
            crate::account_session::finalize_activation(
                &site.profile,
                site.operator.inner(),
                staged,
                &account,
            )
            .await?;
            assert!(
                ask.clone().perform(&served).await.is_err(),
                "new attachment must not revive old mounts"
            );
            ask.perform(&mount_registry(&context).await?).await?;
            Ok(())
        }

        #[tokio::test]
        async fn concurrent_first_use_and_reload_keep_one_peer_identity() -> Result<()> {
            let (_tmp, site) = fixture().await?;
            let site = RegistryContext::from(&site);
            let (left, right) = tokio::join!(endpoint_key(&site), endpoint_key(&site));
            let key = left?.public();
            assert_eq!(key, right?.public());
            assert_eq!(key, endpoint_key(&site).await?.public());
            Ok(())
        }

        #[tokio::test]
        async fn a_corrupt_peer_seed_is_not_silently_replaced() -> Result<()> {
            use dialog_effects::credential::Secret;
            let (_tmp, site) = fixture().await?;
            site.profile
                .credential()
                .site(dialog_capability::SiteId::from(PEER_SEED_SITE.to_owned()))
                .save(Secret::from(vec![42; 3]))
                .perform(&site.operator)
                .await?;
            let error = endpoint_key(&RegistryContext::from(&site))
                .await
                .unwrap_err();
            assert!(error.to_string().contains("corrupt"));
            let stored = site
                .profile
                .credential()
                .site(dialog_capability::SiteId::from(PEER_SEED_SITE.to_owned()))
                .load::<Secret>()
                .perform(&site.operator)
                .await?;
            assert_eq!(stored.as_bytes(), &[42; 3]);
            Ok(())
        }
    }
}

#[cfg(feature = "rtc")]
pub use serving::{Served, serve};
