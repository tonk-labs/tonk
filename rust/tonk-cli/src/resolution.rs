//! Durable DID evidence for local capability verification.
//!
//! Refresh network-resolved documents whenever available; reuse verified
//! evidence without an age limit only on unavailability. A current refusal or
//! invalid document clears old evidence durably. Signature verification and
//! document/controller/fragment binding remain Dialog's, not a second parser.

use dialog_capability::Provider;
use dialog_did_web::{DidDocument, DidKeyProvider, MultiVerifier, Resolve, ResolveError};
use dialog_varsig::Did;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_ENTRIES: usize = 128;
const MAX_CACHE_BYTES: usize = 8 * 1024 * 1024;
const CACHE_FILE: &str = "documents-v1.json";

pub(crate) enum Fetched {
    Document(Vec<u8>),
    Unavailable,
    Refused(String),
}

#[async_trait::async_trait]
pub(crate) trait Fetch: Send + Sync {
    async fn fetch(&self, url: &str) -> Fetched;
}

#[derive(Clone)]
pub(crate) struct Http(reqwest::Client);

#[async_trait::async_trait]
impl Fetch for Http {
    async fn fetch(&self, url: &str) -> Fetched {
        let mut response = match self.0.get(url).send().await {
            Ok(response) => response,
            Err(_) => return Fetched::Unavailable,
        };
        if response.status().is_server_error()
            || response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
        {
            return Fetched::Unavailable;
        }
        if !response.status().is_success() {
            return Fetched::Refused(format!("DID document returned {}", response.status()));
        }
        let limit = dialog_did_web::MAX_DOCUMENT_BYTES;
        if response
            .content_length()
            .is_some_and(|length| length > limit as u64)
        {
            return Fetched::Refused("oversized DID document".into());
        }
        let mut body = Vec::new();
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    if body.len().saturating_add(chunk.len()) > limit {
                        return Fetched::Refused("oversized DID document".into());
                    }
                    body.extend_from_slice(&chunk);
                }
                Ok(None) => return Fetched::Document(body),
                Err(_) => return Fetched::Unavailable,
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Documents {
    version: u8,
    // None is a durable refusal, not an absent/uninitialized entry.
    entries: BTreeMap<String, Option<String>>,
}

impl Default for Documents {
    fn default() -> Self {
        Self {
            version: 1,
            entries: BTreeMap::new(),
        }
    }
}

fn failure(error: impl std::fmt::Display) -> ResolveError {
    ResolveError::Fetch(format!("DID evidence unavailable: {error}"))
}

fn read(directory: &Path) -> Result<Documents, ResolveError> {
    let file = match std::fs::File::open(directory.join(CACHE_FILE)) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Documents::default());
        }
        Err(error) => return Err(failure(error)),
    };
    let mut bytes = Vec::new();
    file.take(MAX_CACHE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(failure)?;
    if bytes.len() > MAX_CACHE_BYTES {
        return Err(failure("oversized cache"));
    }
    let documents: Documents = serde_json::from_slice(&bytes)
        .map_err(|error| failure(format!("corrupt cache; refusing to replace it: {error}")))?;
    if documents.version != 1 || documents.entries.len() > MAX_ENTRIES {
        return Err(failure("unsupported or oversized cache"));
    }
    Ok(documents)
}

fn verified(body: &str, did: &Did, fragment: Option<&str>) -> Result<MultiVerifier, ResolveError> {
    let document: DidDocument = serde_json::from_str(body)
        .map_err(|error| ResolveError::MalformedDocument(error.to_string()))?;
    document.verifier(did, fragment)
}

fn cached(
    documents: &Documents,
    did: &Did,
    fragment: Option<&str>,
) -> Result<MultiVerifier, ResolveError> {
    match documents.entries.get(did.as_str()) {
        Some(Some(body)) => verified(body, did, fragment),
        Some(None) => Err(failure(
            "the latest online document was refused; old keys cannot be reused",
        )),
        None => Err(failure("no previously verified document for this DID")),
    }
}

#[derive(Clone)]
pub(crate) struct Cached<F = Http> {
    directory: PathBuf,
    fetch: Arc<F>,
    requests: Arc<tokio::sync::Semaphore>,
    refresh: Arc<tokio::sync::Mutex<()>>,
}

impl Cached {
    pub(crate) fn new(directory: PathBuf) -> Result<Self, ResolveError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .map_err(failure)?;
        Ok(Self::with_fetch(directory, Http(client)))
    }
}

impl<F: Fetch> Cached<F> {
    pub(crate) fn with_fetch(directory: PathBuf, fetch: F) -> Self {
        Self {
            directory,
            fetch: Arc::new(fetch),
            requests: Arc::new(tokio::sync::Semaphore::new(8)),
            refresh: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    async fn load(&self) -> Result<Documents, ResolveError> {
        let directory = self.directory.clone();
        tokio::task::spawn_blocking(move || read(&directory))
            .await
            .map_err(failure)?
    }

    /// Serialize refresh across processes, without blocking an executor or
    /// queuing unbounded filesystem tasks. Contention can reuse only evidence
    /// already persisted by a completed refresh, never an unverified response.
    async fn lock(&self) -> Result<Option<std::fs::File>, ResolveError> {
        let directory = self.directory.clone();
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&directory).map_err(failure)?;
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(directory.join("documents.lock"))
                .map_err(failure)?;
            match file.try_lock() {
                Ok(()) => Ok(Some(file)),
                Err(std::fs::TryLockError::WouldBlock) => Ok(None),
                Err(error) => Err(failure(error)),
            }
        })
        .await
        .map_err(failure)?
    }

    async fn save(
        &self,
        mut documents: Documents,
        did: &Did,
        body: Option<String>,
    ) -> Result<(), ResolveError> {
        let directory = self.directory.clone();
        let did = did.to_string();
        tokio::task::spawn_blocking(move || {
            documents.entries.insert(did.clone(), body);
            let bytes = loop {
                let bytes = serde_json::to_vec(&documents).map_err(failure)?;
                if documents.entries.len() <= MAX_ENTRIES && bytes.len() <= MAX_CACHE_BYTES {
                    break bytes;
                }
                let remove = documents
                    .entries
                    .keys()
                    .find(|key| **key != did)
                    .cloned()
                    .ok_or_else(|| failure("DID document cannot fit in bounded cache"))?;
                documents.entries.remove(&remove);
            };
            let mut file = tempfile::NamedTempFile::new_in(&directory).map_err(failure)?;
            file.write_all(&bytes).map_err(failure)?;
            file.flush().map_err(failure)?;
            file.as_file().sync_all().map_err(failure)?;
            file.persist(directory.join(CACHE_FILE)).map_err(failure)?;
            #[cfg(unix)]
            std::fs::File::open(&directory)
                .and_then(|directory| directory.sync_all())
                .map_err(failure)?;
            Ok(())
        })
        .await
        .map_err(failure)?
    }
}

#[async_trait::async_trait]
impl<F: Fetch + 'static> Provider<Resolve> for Cached<F> {
    async fn execute(&self, input: Resolve) -> Result<MultiVerifier, ResolveError> {
        if input.did.method() == "key" {
            return DidKeyProvider.execute(input).await;
        }
        if input.did.as_str().len() > 2048 {
            return Err(ResolveError::MalformedDid("DID exceeds size limit".into()));
        }
        let (base, fragment) = input
            .did
            .as_str()
            .split_once('#')
            .map_or((input.did.as_str(), None), |(base, fragment)| {
                (base, Some(fragment))
            });
        let did: Did = base.parse().map_err(failure)?;
        let url = match did.method() {
            "web" => dialog_did_web::did_web_url(base)?,
            "plc" => dialog_did_web::did_plc_url(base)?,
            method => return Err(ResolveError::UnsupportedMethod(method.into())),
        };
        let Ok(_permit) = self.requests.try_acquire() else {
            return cached(&self.load().await?, &did, fragment);
        };
        let Ok(_refresh) =
            tokio::time::timeout(std::time::Duration::from_secs(10), self.refresh.lock()).await
        else {
            return cached(&self.load().await?, &did, fragment);
        };
        let Some(_lock) = self.lock().await? else {
            return cached(&self.load().await?, &did, fragment);
        };
        let documents = self.load().await?;
        match self.fetch.fetch(&url).await {
            Fetched::Unavailable => cached(&documents, &did, fragment),
            Fetched::Refused(reason) => {
                self.save(documents, &did, None).await?;
                Err(failure(reason))
            }
            Fetched::Document(bytes) => {
                if bytes.len() > dialog_did_web::MAX_DOCUMENT_BYTES {
                    self.save(documents, &did, None).await?;
                    return Err(failure("oversized DID document"));
                }
                let body = String::from_utf8(bytes)
                    .map_err(|error| ResolveError::MalformedDocument(error.to_string()));
                // Validate the whole document before caching it. A missing
                // requested fragment must not poison other valid member keys.
                let validated = body.and_then(|body| {
                    verified(&body, &did, None)?;
                    Ok(body)
                });
                match validated {
                    Ok(body) => {
                        let result = verified(&body, &did, fragment);
                        self.save(documents, &did, Some(body)).await?;
                        result
                    }
                    Err(error) => {
                        self.save(documents, &did, None).await?;
                        Err(error)
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU8, Ordering};
    const DID: &str = "did:web:account.example";
    const FIRST: &str = "z6MkrF2Jq3mNhFsEtYvQeTVZQfZ5fFPMj3DcbSt9uhzNcoVR";
    const SECOND: &str = "z6MkuGiBdtP3ZdjU6H9fsvKJt6PJPMgpCsM9sYswpQvFQ3Pa";
    struct Mock(Arc<AtomicU8>);
    #[async_trait::async_trait]
    impl Fetch for Mock {
        async fn fetch(&self, url: &str) -> Fetched {
            assert_eq!(url, "https://account.example/.well-known/did.json");
            let mode = self.0.load(Ordering::SeqCst);
            let key = match mode {
                0 => FIRST,
                1 => return Fetched::Unavailable,
                2 => SECOND,
                3 => return Fetched::Refused("410 Gone".into()),
                4 => return Fetched::Document(b"invalid JSON".to_vec()),
                _ => FIRST,
            };
            Fetched::Document(serde_json::to_vec(&serde_json::json!({
                "id": if mode == 5 { "did:web:someone-else.example" } else { DID },
                "verificationMethod": [{ "id": format!("{DID}#current"), "controller": DID, "type": "Multikey", "publicKeyMultibase": key }],
            })).unwrap())
        }
    }

    #[tokio::test]
    async fn concurrent_first_use_does_not_mistake_an_in_flight_refresh_for_missing_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let resolver =
            Cached::with_fetch(directory.path().into(), Mock(Arc::new(AtomicU8::new(0))));
        let (left, right) = tokio::join!(
            Resolve::new(DID.parse().unwrap()).perform(&resolver),
            Resolve::new(DID.parse().unwrap()).perform(&resolver),
        );
        assert_eq!(left.unwrap(), right.unwrap());
    }

    #[tokio::test]
    async fn http_adapter_distinguishes_outage_from_refusal_and_bounds_documents() {
        use axum::{Router, http::StatusCode, routing::get};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new()
            .route(
                "/unavailable",
                get(|| async { StatusCode::SERVICE_UNAVAILABLE }),
            )
            .route("/gone", get(|| async { StatusCode::GONE }))
            .route(
                "/redirect",
                get(|| async { (StatusCode::FOUND, [("location", "/valid")]) }),
            )
            .route(
                "/oversized",
                get(|| async { vec![b'x'; dialog_did_web::MAX_DOCUMENT_BYTES + 1] }),
            )
            .route("/valid", get(|| async { "document" }));
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let directory = tempfile::tempdir().unwrap();
        let resolver = Cached::new(directory.path().into()).unwrap();
        assert!(matches!(
            resolver.fetch.fetch(&format!("{origin}/unavailable")).await,
            Fetched::Unavailable
        ));
        for path in ["gone", "redirect", "oversized"] {
            assert!(
                matches!(
                    resolver.fetch.fetch(&format!("{origin}/{path}")).await,
                    Fetched::Refused(_)
                ),
                "{path}"
            );
        }
        assert!(matches!(
            resolver.fetch.fetch(&format!("{origin}/valid")).await,
            Fetched::Document(_)
        ));
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn offline_restart_uses_verified_keys_but_online_rotation_replaces_them() {
        let directory = tempfile::tempdir().unwrap();
        let mode = Arc::new(AtomicU8::new(1));
        let resolver = || Cached::with_fetch(directory.path().into(), Mock(mode.clone()));
        let resolve = || Resolve::new(DID.parse().unwrap());
        assert!(resolve().perform(&resolver()).await.is_err());
        mode.store(0, Ordering::SeqCst);
        let first = resolve().perform(&resolver()).await.unwrap();
        mode.store(1, Ordering::SeqCst);
        assert_eq!(first, resolve().perform(&resolver()).await.unwrap());
        mode.store(2, Ordering::SeqCst);
        let second = resolve().perform(&resolver()).await.unwrap();
        assert_ne!(first, second);
        mode.store(1, Ordering::SeqCst);
        assert_eq!(second, resolve().perform(&resolver()).await.unwrap());
    }

    #[tokio::test]
    async fn online_refusal_or_invalid_binding_cannot_resurrect_an_old_key_offline() {
        for invalid in [3, 4, 5] {
            let directory = tempfile::tempdir().unwrap();
            let mode = Arc::new(AtomicU8::new(0));
            let resolver = || Cached::with_fetch(directory.path().into(), Mock(mode.clone()));
            Resolve::new(DID.parse().unwrap())
                .perform(&resolver())
                .await
                .unwrap();
            mode.store(invalid, Ordering::SeqCst);
            assert!(
                Resolve::new(DID.parse().unwrap())
                    .perform(&resolver())
                    .await
                    .is_err()
            );
            mode.store(1, Ordering::SeqCst);
            assert!(
                Resolve::new(DID.parse().unwrap())
                    .perform(&resolver())
                    .await
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn fragment_selection_and_corrupt_cache_fail_closed_without_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let mode = Arc::new(AtomicU8::new(0));
        let resolver = Cached::with_fetch(directory.path().into(), Mock(mode.clone()));
        Resolve::new(format!("{DID}#current").parse().unwrap())
            .perform(&resolver)
            .await
            .unwrap();
        mode.store(1, Ordering::SeqCst);
        assert!(
            Resolve::new(format!("{DID}#removed").parse().unwrap())
                .perform(&resolver)
                .await
                .is_err()
        );
        std::fs::write(directory.path().join(CACHE_FILE), b"corrupt").unwrap();
        mode.store(0, Ordering::SeqCst);
        assert!(
            Resolve::new(DID.parse().unwrap())
                .perform(&resolver)
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read(directory.path().join(CACHE_FILE)).unwrap(),
            b"corrupt"
        );
    }
}
