//! Revocation verification for local stores, independent of their transport.
//!
//! Always refresh from configured access services when reachable. Only an
//! unavailable lookup may reuse a previously verified answer, without a TTL.
//! Unknown answers fail closed. Learned revocations are persisted atomically,
//! merged under a cross-process lock, and never overwritten by older approvals.

use dialog_ucan_core::revocation::{RevocationChecker, RevocationMatch, RevocationSelector};
use std::{
    io::{Read, Write},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tonk_identity::revocation::evidence::{Answer, Evidence, MAX_RESPONSE_BYTES, Query, Verdict};

const MAX_CACHE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
#[error("revocation verification unavailable: {0}")]
pub(crate) struct Error(String);

#[derive(Clone)]
pub(crate) struct Cached {
    directory: PathBuf,
    sources: Vec<String>,
    client: reqwest::Client,
    requests: Arc<tokio::sync::Semaphore>,
}

enum LookupError {
    Unavailable,
    Invalid(String),
}

impl Cached {
    /// Trust only access services already configured in local account/space
    /// metadata. A caller must never nominate the service approving its chain.
    #[cfg(feature = "rtc")]
    pub(crate) async fn for_registry(
        profile: &dialog_operator::Profile,
        operator: &dialog_operator::Operator<dialog_storage::provider::storage::NativeSpace>,
        store: &crate::space::SpaceStore,
        offers: &[dialog_effects::peer::Offer],
    ) -> anyhow::Result<Self> {
        use dialog_capability::Subject;
        use dialog_remote_ucan_s3::UcanAddress;
        use dialog_repository::RepositoryMemoryExt;

        let mut endpoints = std::collections::BTreeSet::new();
        if let Some(provider) = crate::account::stored_provider_in(profile, operator, store).await?
        {
            endpoints.insert(UcanAddress::new(provider.address()).endpoint().to_string());
        }
        for offer in offers {
            let meta = Subject::from(offer.subject.clone())
                .branch("meta")
                .open()
                .perform(operator)
                .await?;
            endpoints.extend(
                tonk_schema::directory::access_endpoints(&meta, operator)
                    .await
                    .map_err(|error| {
                        anyhow::anyhow!("cannot read revocation services: {error:?}")
                    })?,
            );
        }
        Self::new(store.root().join("revocations"), endpoints).map_err(Into::into)
    }

    #[cfg(feature = "rtc")]
    pub(crate) fn has_sources(&self) -> bool {
        !self.sources.is_empty()
    }

    pub(crate) fn new(
        directory: PathBuf,
        endpoints: impl IntoIterator<Item = String>,
    ) -> Result<Self, Error> {
        let sources: std::collections::BTreeSet<_> = endpoints
            .into_iter()
            .map(|endpoint| source_url(&endpoint))
            .collect::<Result<_, _>>()?;
        if sources.len() > 16 {
            return Err(Error("too many revocation services (maximum 16)".into()));
        }
        Ok(Self {
            directory,
            sources: sources.into_iter().collect(),
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(3))
                .build()
                .map_err(|e| Error(e.to_string()))?,
            requests: Arc::new(tokio::sync::Semaphore::new(8)),
        })
    }

    /// Never hold the filesystem lock over a network operation. Merge into a
    /// freshly read cache after each response so other processes' revocations
    /// cannot be lost by a late positive result.
    async fn cache<T: Send + 'static>(
        &self,
        write: bool,
        update: impl FnOnce(&mut Evidence) -> Result<T, Error> + Send + 'static,
    ) -> Result<T, Error> {
        let directory = self.directory.clone();
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&directory)
                .map_err(|e| Error(format!("cache directory: {e}")))?;
            let lock = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(directory.join("evidence.lock"))
                .map_err(|e| Error(format!("cache lock: {e}")))?;
            lock.lock().map_err(|e| Error(format!("cache lock: {e}")))?;
            let path = directory.join("evidence-v1.json");
            let mut evidence = match std::fs::File::open(&path) {
                Ok(file) => {
                    let mut bytes = Vec::new();
                    file.take(MAX_CACHE_BYTES + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|e| Error(format!("cache read: {e}")))?;
                    if bytes.len() as u64 > MAX_CACHE_BYTES {
                        return Err(Error("revocation cache is too large".into()));
                    }
                    let evidence: Evidence = serde_json::from_slice(&bytes).map_err(|e| {
                        Error(format!("corrupt cache; refusing to replace it: {e}"))
                    })?;
                    evidence.validate().map_err(|e| Error(e.to_string()))?;
                    evidence
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Evidence::default(),
                Err(error) => return Err(Error(format!("cache read: {error}"))),
            };
            let result = update(&mut evidence)?;
            if write {
                let mut temporary = tempfile::NamedTempFile::new_in(&directory)
                    .map_err(|e| Error(format!("cache temporary file: {e}")))?;
                serde_json::to_writer(&mut temporary, &evidence)
                    .map_err(|e| Error(format!("cache encode: {e}")))?;
                temporary
                    .flush()
                    .map_err(|e| Error(format!("cache flush: {e}")))?;
                temporary
                    .as_file()
                    .sync_all()
                    .map_err(|e| Error(format!("cache sync: {e}")))?;
                temporary
                    .persist(&path)
                    .map_err(|e| Error(format!("cache persist: {e}")))?;
                #[cfg(unix)]
                std::fs::File::open(&directory)
                    .and_then(|directory| directory.sync_all())
                    .map_err(|e| Error(format!("cache directory sync: {e}")))?;
            }
            Ok(result)
        })
        .await
        .map_err(|e| Error(format!("cache task: {e}")))?
    }

    async fn lookup(&self, source: &str, query: &Query) -> Result<Answer, LookupError> {
        // Overload is not permission. A cached answer can sustain offline or
        // overloaded service operation; a new chain still needs evidence.
        let _permit = self
            .requests
            .try_acquire()
            .map_err(|_| LookupError::Unavailable)?;
        let mut response = self
            .client
            .post(source)
            .json(query)
            .send()
            .await
            .map_err(|_| LookupError::Unavailable)?;
        if response.status().is_server_error()
            || response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
        {
            return Err(LookupError::Unavailable);
        }
        if !response.status().is_success() {
            return Err(LookupError::Invalid(format!(
                "service returned {}; it must support /ucan/revocations",
                response.status()
            )));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err(LookupError::Invalid("oversized response".into()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| LookupError::Unavailable)?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(LookupError::Invalid("oversized response".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        let answer: Answer = serde_json::from_slice(&bytes)
            .map_err(|e| LookupError::Invalid(format!("malformed response: {e}")))?;
        answer
            .validate_for(query)
            .map_err(|e| LookupError::Invalid(e.to_string()))?;
        Ok(answer)
    }
}

fn source_url(endpoint: &str) -> Result<String, Error> {
    let mut url =
        url::Url::parse(endpoint).map_err(|e| Error(format!("invalid service URL: {e}")))?;
    let loopback = url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if !(url.scheme() == "https" || url.scheme() == "http" && loopback)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error("revocation service requires HTTPS (or development loopback), without credentials, query, or fragment".into()));
    }
    url.set_path(&format!("{}/revocations", url.path().trim_end_matches('/')));
    Ok(url.into())
}

impl RevocationChecker for Cached {
    type Error = Error;

    async fn query(
        &self,
        selector: RevocationSelector<'_>,
    ) -> Result<Option<RevocationMatch>, Error> {
        if selector.by.is_empty() {
            return Ok(None);
        }
        let query =
            Query::new(selector.delegation, selector.by).map_err(|e| Error(e.to_string()))?;
        let q = query.clone();
        if let Verdict::Revoked(principal) = self
            .cache(false, move |evidence| Ok(evidence.verdict("", &q)))
            .await?
        {
            return Ok(Some(RevocationMatch {
                revocation: selector.delegation,
                principal: principal
                    .parse()
                    .map_err(|_| Error("invalid cached revoker".into()))?,
            }));
        }
        if self.sources.is_empty() {
            return Err(Error("no trusted revocation service is configured".into()));
        }

        let mut unavailable = false;
        for source in &self.sources {
            let result = self.lookup(source, &query).await;
            let q = query.clone();
            let source = source.clone();
            let verdict = match result {
                Ok(answer) => {
                    self.cache(true, move |evidence| {
                        evidence
                            .record(&source, &q, &answer)
                            .map_err(|e| Error(e.to_string()))?;
                        Ok(evidence.verdict(&source, &q))
                    })
                    .await?
                }
                Err(LookupError::Unavailable) => {
                    self.cache(false, move |evidence| Ok(evidence.verdict(&source, &q)))
                        .await?
                }
                Err(LookupError::Invalid(detail)) => return Err(Error(detail)),
            };
            match verdict {
                Verdict::Revoked(principal) => {
                    return Ok(Some(RevocationMatch {
                        revocation: selector.delegation,
                        principal: principal
                            .parse()
                            .map_err(|_| Error("invalid cached revoker".into()))?,
                    }));
                }
                Verdict::Approved => {}
                // Still query other services: one can know it is revoked even
                // when another is down and we have no previous approval.
                Verdict::Unknown => unavailable = true,
            }
        }
        if unavailable {
            Err(Error(
                "no cached evidence for an unavailable service".into(),
            ))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, extract::State, response::IntoResponse, routing::post};
    use std::sync::atomic::{AtomicU8, Ordering};
    const CID: &str = "bafyreidyasztqnjah3v2s5vr4sidcsgksbfvcgeegc4p37irqvps7a7jd4";
    const ALICE: &str = "did:key:z6MkrF2Jq3mNhFsEtYvQeTVZQfZ5fFPMj3DcbSt9uhzNcoVR";

    async fn service(
        State(mode): State<Arc<AtomicU8>>,
        Json(query): Json<Query>,
    ) -> axum::response::Response {
        let revoked_by = match mode.load(Ordering::SeqCst) {
            1 => return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response(),
            2 => query.by.clone(),
            3 => return (axum::http::StatusCode::OK, "malformed").into_response(),
            _ => Default::default(),
        };
        let targets = Some(if revoked_by.is_empty() {
            Default::default()
        } else {
            [query.delegation.clone()].into_iter().collect()
        });
        Json(Answer {
            query,
            revoked_by,
            targets,
        })
        .into_response()
    }

    async fn fixture() -> anyhow::Result<(tempfile::TempDir, crate::site::TonkSite)> {
        let directory = tempfile::tempdir()?;
        let store = crate::space::SpaceStore::at(directory.path().join("state"));
        std::fs::create_dir_all(store.root())?;
        let site = crate::site::TonkSite::init_with(
            directory.path(),
            crate::site::SiteConfig {
                profile_name: "revocation-write-test".into(),
                profile_directory: dialog_effects::storage::Directory::At(
                    directory
                        .path()
                        .join("profile")
                        .to_string_lossy()
                        .into_owned(),
                ),
                require_account: false,
                provision_account_spaces: false,
                account_store: store,
            },
        )
        .await?;
        Ok((directory, site))
    }

    /// A real signature chain through a network-resolved DID, not a resolver
    /// response tested in isolation. The offline half recreates both caches
    /// and mints new delegation CIDs; cached absence must not skip any of the
    /// other authorization checks or authorize substituted content.
    #[tokio::test]
    async fn offline_did_and_revocation_evidence_preserves_all_authorization_checks()
    -> anyhow::Result<()> {
        use crate::resolution::{Fetch, Fetched};
        use dialog_capability::Subject;
        use dialog_common::Buffer;
        use dialog_credentials::{Ed25519Signer, Signer};
        use dialog_effects::{Use, archive};
        use dialog_iroh_remote::{
            serve::Responder,
            wire::{Refusal, Response},
        };
        use dialog_ucan::Scope;
        use dialog_ucan_core::container::bundle::InvocationBundle;
        use dialog_ucan_core::subject::Subject as DelegatedSubject;
        use dialog_ucan_core::time::{Timestamp, timestamp::SystemTime};
        use dialog_ucan_core::{Container, DelegationBuilder, InvocationBuilder, InvocationChain};
        use dialog_varsig::{Did, Principal};
        use std::collections::HashMap;

        const WEB_DID: &str = "did:web:rtc-authority.example";
        struct Documents {
            key: String,
            mode: Arc<AtomicU8>,
        }
        #[async_trait::async_trait]
        impl Fetch for Documents {
            async fn fetch(&self, url: &str) -> Fetched {
                assert_eq!(url, "https://rtc-authority.example/.well-known/did.json");
                if self.mode.load(Ordering::SeqCst) == 1 {
                    return Fetched::Unavailable;
                }
                Fetched::Document(serde_json::to_vec(&serde_json::json!({
                    "id": WEB_DID,
                    "verificationMethod": [{ "id": format!("{WEB_DID}#key"), "controller": WEB_DID,
                        "type": "Multikey", "publicKeyMultibase": self.key }],
                })).unwrap())
            }
        }

        let (directory, site) = fixture().await?;
        let mode = Arc::new(AtomicU8::new(0));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = format!("http://{}/ucan/", listener.local_addr()?);
        let router = Router::new()
            .route("/ucan/revocations", post(service))
            .with_state(mode.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let signer = Ed25519Signer::import(&[73; 32]).await?;
        let impostor = Ed25519Signer::import(&[74; 32]).await?;
        let did: Did = WEB_DID.parse()?;
        let subject = site.profile.did();
        let past = Timestamp::new(SystemTime::now() - Duration::from_secs(300))?;
        let future = Timestamp::new(SystemTime::now() + Duration::from_secs(300))?;
        let mut online_proofs = std::collections::HashSet::new();
        for offline in [false, true] {
            mode.store(u8::from(offline), Ordering::SeqCst);
            for case in [
                "allowed",
                "wrong subject",
                "read only",
                "expired invocation",
                "expired proof",
                "bad signature",
                "substituted payload",
            ] {
                let block = Buffer::from(format!("{offline}: {case}").into_bytes());
                let catalog = Subject::from(subject.clone())
                    .attenuate(Use)
                    .attenuate(archive::Archive)
                    .attenuate(archive::Catalog::new("acceptance"));
                let capability = catalog.clone().invoke(archive::Put::new(block.clone()));
                let scope = Scope::invoke(&capability);
                let command = scope.command.segments().clone();
                let grant = DelegationBuilder::new()
                    .issuer(site.profile.signer().signer().clone())
                    .audience(&did)
                    .subject(DelegatedSubject::Specific(if case == "wrong subject" {
                        ALICE.parse()?
                    } else {
                        subject.clone()
                    }))
                    .command(if case == "read only" {
                        vec!["use".into(), "get".into(), "archive".into(), "block".into()]
                    } else {
                        command.clone()
                    })
                    .expiration(if case == "expired proof" {
                        past
                    } else {
                        future
                    })
                    .try_build()
                    .await?;
                let proof = grant.to_cid();
                if offline {
                    assert!(
                        !online_proofs.contains(&proof),
                        "offline coverage must use newly minted proofs"
                    );
                } else {
                    online_proofs.insert(proof);
                }
                let invocation = InvocationBuilder::new()
                    .issuer(
                        Signer::from(if case == "bad signature" {
                            impostor.clone()
                        } else {
                            signer.clone()
                        })
                        .with_did(did.clone()),
                    )
                    .audience(&subject)
                    .subject(&subject)
                    .command(command)
                    .arguments(scope.parameters.args())
                    .proofs(vec![proof])
                    .expiration(if case == "expired invocation" {
                        past
                    } else {
                        future
                    })
                    .try_build()
                    .await?;
                let chain =
                    InvocationChain::new(invocation, HashMap::from([(proof, Arc::new(grant))]));
                let body = if case == "substituted payload" {
                    b"unsigned replacement".to_vec()
                } else {
                    block.as_ref().to_vec()
                };
                let bundle = InvocationBundle::from_chain(&chain, [body.clone()])?;
                let request = Container::from(&bundle).into_bytes()?;
                let resolver = crate::resolution::Cached::with_fetch(
                    directory.path().join("did-cache"),
                    Documents {
                        key: signer
                            .did()
                            .to_string()
                            .trim_start_matches("did:key:")
                            .into(),
                        mode: mode.clone(),
                    },
                );
                let response = Responder::new(site.operator.inner().clone(), resolver)
                    .with_revocations(Cached::new(
                        directory.path().join("evidence"),
                        [endpoint.clone()],
                    )?)
                    .answer(&request)
                    .await
                    .without_stream();
                match (&response, case) {
                    (Response::Performed(bytes), "allowed") => {
                        dialog_iroh_remote::wire::decode::<Result<(), archive::ArchiveError>>(
                            "put", bytes,
                        )??;
                    }
                    (
                        Response::Refused(Refusal::Malformed(_) | Refusal::Unauthorized(_)),
                        "substituted payload",
                    ) => {}
                    (Response::Refused(Refusal::Unauthorized(_)), case) if case != "allowed" => {}
                    _ => anyhow::bail!("unexpected {case}, offline={offline}: {response:?}"),
                }
                // Check both the signed address and the substituted bytes;
                // neither may have reached storage after a refusal.
                for digest in [
                    block.blake3_hash().clone(),
                    Buffer::from(body).blake3_hash().clone(),
                ] {
                    let stored = catalog
                        .clone()
                        .invoke(archive::Get::new(digest))
                        .perform(site.operator.inner())
                        .await?;
                    assert_eq!(
                        stored.as_deref(),
                        if case == "allowed" {
                            Some(block.as_ref())
                        } else {
                            None
                        },
                        "{case}, offline={offline}"
                    );
                }
            }
        }
        server.abort();
        let _ = server.await;
        Ok(())
    }

    #[tokio::test]
    async fn refresh_learns_revocation_and_restart_never_resurrects_cached_approval() {
        let directory = tempfile::tempdir().unwrap();
        let mode = Arc::new(AtomicU8::new(1));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/ucan/", listener.local_addr().unwrap());
        let router = Router::new()
            .route("/ucan/revocations", post(service))
            .with_state(mode.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let cache = || Cached::new(directory.path().into(), [endpoint.clone()]).unwrap();
        let checker = cache();
        let by = [ALICE.parse().unwrap()];
        let selector = || RevocationSelector::new(CID.parse().unwrap(), &by);
        assert!(
            checker.query(selector()).await.is_err(),
            "offline without evidence is not approval"
        );
        mode.store(0, Ordering::SeqCst);
        assert!(checker.query(selector()).await.unwrap().is_none());
        mode.store(1, Ordering::SeqCst);
        assert!(
            cache().query(selector()).await.unwrap().is_none(),
            "restart uses verified offline evidence"
        );
        mode.store(3, Ordering::SeqCst);
        assert!(
            checker.query(selector()).await.is_err(),
            "malformed online evidence is not an offline answer"
        );
        mode.store(2, Ordering::SeqCst);
        assert!(checker.query(selector()).await.unwrap().is_some());
        mode.store(0, Ordering::SeqCst);
        assert!(
            cache().query(selector()).await.unwrap().is_some(),
            "an older online answer cannot revive it"
        );
        server.abort();
        assert!(
            cache().query(selector()).await.unwrap().is_some(),
            "offline restart retains the revocation"
        );
    }

    #[tokio::test]
    async fn verified_responder_stops_writes_after_refresh_and_offline_restart()
    -> anyhow::Result<()> {
        use dialog_capability::{Fork, SiteFork, Subject};
        use dialog_common::Buffer;
        use dialog_effects::{Use, archive};
        use dialog_iroh_remote::{
            serve::Responder,
            site::{Iroh, IrohFork},
            wire::Response,
        };

        let (directory, site) = fixture().await?;
        let mode = Arc::new(AtomicU8::new(1));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = format!("http://{}/ucan/", listener.local_addr()?);
        let router = Router::new()
            .route("/ucan/revocations", post(service))
            .with_state(mode.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let responder = || {
            Responder::new(
                site.operator.inner().clone(),
                dialog_did_web::CachingResolver::new(dialog_did_web::WebResolver::new()),
            )
            .with_revocations(
                Cached::new(directory.path().join("cache"), [endpoint.clone()]).unwrap(),
            )
        };

        for (label, service_mode, allowed) in [
            ("unknown offline", 1, false),
            ("fresh online", 0, true),
            ("cached offline", 1, true),
            ("revoked online", 2, false),
            ("old positive", 0, false),
            ("revoked offline", 1, false),
        ] {
            mode.store(service_mode, Ordering::SeqCst);
            let block = Buffer::from(label.as_bytes().to_vec());
            let catalog = Subject::from(site.profile.did())
                .attenuate(Use)
                .attenuate(archive::Archive)
                .attenuate(archive::Catalog::new("revocation-test"));
            let fork: IrohFork<archive::Put> = Fork::<Iroh, _>::new(
                catalog.clone().invoke(archive::Put::new(block.clone())),
                ALICE.parse()?,
            )
            .into();
            let request = fork.authorize(site.operator.inner()).await?;
            // Recreate the checker on every request, exercising durable evidence
            // rather than an in-memory approval/denial carried between calls.
            let reply = responder()
                .answer(request.authorization.as_bytes())
                .await
                .without_stream();
            match reply {
                Response::Performed(bytes) if allowed => {
                    dialog_iroh_remote::wire::decode::<Result<(), archive::ArchiveError>>(
                        "archive result",
                        &bytes,
                    )??;
                }
                Response::Refused(_) if !allowed => {}
                other => anyhow::bail!("unexpected response for {label}: {other:?}"),
            }
            let stored = catalog
                .invoke(archive::Get::new(block.blake3_hash().clone()))
                .perform(site.operator.inner())
                .await;
            if allowed {
                assert_eq!(stored?.as_deref(), Some(block.as_ref()), "{label}");
            } else {
                assert!(
                    matches!(stored, Ok(None)),
                    "rejected write changed storage: {label}: {stored:?}"
                );
            }
        }

        // Public self-signed discovery passes the same strict verifier even
        // without configured revocation services; it grants no content rights.
        // Rotation creates both a new operator key and a new delegation CID.
        // The previous operator grant was revoked, not the profile itself.
        // A complete cached target snapshot must permit independently valid
        // fresh sessions offline without reusing or exempting that old grant.
        use dialog_operator::DeriveOperator;
        mode.store(1, Ordering::SeqCst);
        for generation in 0..3 {
            let fresh = site
                .profile
                .derive(format!("offline-session-{generation}").into_bytes())
                .allow(Subject::from(site.profile.did()))
                .build(site.storage.clone())
                .await?;
            assert_ne!(fresh.did(), site.operator.did());
            let block = Buffer::from(format!("fresh offline session {generation}").into_bytes());
            let catalog = Subject::from(site.profile.did())
                .attenuate(Use)
                .attenuate(archive::Archive)
                .attenuate(archive::Catalog::new("revocation-test"));
            let fork: IrohFork<archive::Put> = Fork::<Iroh, _>::new(
                catalog.clone().invoke(archive::Put::new(block.clone())),
                ALICE.parse()?,
            )
            .into();
            let request = fork.authorize(&fresh).await?;
            let Response::Performed(bytes) = responder()
                .answer(request.authorization.as_bytes())
                .await
                .without_stream()
            else {
                anyhow::bail!("fresh offline session was refused");
            };
            dialog_iroh_remote::wire::decode::<Result<(), archive::ArchiveError>>(
                "put result",
                &bytes,
            )??;
            assert_eq!(
                catalog
                    .invoke(archive::Get::new(block.blake3_hash().clone()))
                    .perform(site.operator.inner())
                    .await?
                    .as_deref(),
                Some(block.as_ref())
            );
        }

        let ask = Subject::from(site.profile.did())
            .attenuate(Use)
            .attenuate(dialog_effects::peer::Peer)
            .attenuate(dialog_effects::peer::Spaces);
        let signed = site
            .profile
            .access()
            .claim(ask)
            .invoke()
            .perform(site.operator.inner())
            .await?;
        let bytes = dialog_ucan_core::Container::from(signed.chain()).into_bytes()?;
        let public = Responder::new(
            site.operator.inner().clone(),
            dialog_did_web::WebResolver::new(),
        )
        .with_revocations(Cached::new(directory.path().join("empty-cache"), [])?);
        assert!(matches!(
            public.answer(&bytes).await.without_stream(),
            Response::Performed(_)
        ));
        server.abort();
        let _ = server.await;
        Ok(())
    }

    #[tokio::test]
    async fn corrupt_cache_is_preserved_and_never_treated_as_empty() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("evidence-v1.json");
        std::fs::write(&path, "corrupt").unwrap();
        let checker = Cached::new(directory.path().into(), []).unwrap();
        let by = [ALICE.parse().unwrap()];
        assert!(
            checker
                .query(RevocationSelector::new(CID.parse().unwrap(), &by))
                .await
                .unwrap_err()
                .to_string()
                .contains("corrupt cache")
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), "corrupt");
    }

    #[test]
    fn service_identity_must_be_configured_and_transport_authenticated() {
        assert_eq!(
            source_url("https://example.test/ucan/").unwrap(),
            "https://example.test/ucan/revocations"
        );
        for invalid in [
            "http://example.test/ucan/",
            "https://user:secret@example.test/ucan/",
            "file:///tmp/ucan",
            "https://example.test/ucan/?redirect=other",
        ] {
            assert!(source_url(invalid).is_err());
        }
    }
}
