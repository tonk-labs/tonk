use super::*;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use wasm_bindgen_test::wasm_bindgen_test_configure;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
wasm_bindgen_test_configure!(run_in_service_worker);

/// A session of a fresh agent space: its seed, and the powerline the
/// space delegates to it.
async fn session(space: &Ed25519Signer, mcp_session: &str, remote: &url::Url) -> WriterSession {
    let seed = Zeroizing::new(rand::random::<[u8; 32]>());
    session_with(space, seed, mcp_session, remote).await
}

async fn session_with(
    space: &Ed25519Signer,
    seed: Zeroizing<[u8; 32]>,
    mcp_session: &str,
    remote: &url::Url,
) -> WriterSession {
    let key = Ed25519Signer::import(&*seed).await.unwrap();
    let powerline = tonk_identity::delegation::mint_addressed_device_delegation(
        space.clone(),
        &key.did(),
        remote,
    )
    .await
    .unwrap();
    WriterSession {
        seed,
        powerline,
        mcp_session: mcp_session.to_owned(),
        home: remote.clone(),
    }
}

#[dialog_common::test]
async fn a_session_id_derives_one_writer_key() {
    let space = Ed25519Signer::generate().await.unwrap();
    let remote = url::Url::parse("http://127.0.0.1:1/ucan/").unwrap();
    let seed = rand::random::<[u8; 32]>();
    let first = session_with(&space, Zeroizing::new(seed), "mcp-1", &remote).await;
    let again = session_with(&space, Zeroizing::new(seed), "mcp-1", &remote).await;
    let other = session_with(&space, Zeroizing::new(seed), "mcp-2", &remote).await;
    let elsewhere = session(&space, "mcp-1", &remote).await;

    let first = WriterKey::derive(&first).await.unwrap();
    let again = WriterKey::derive(&again).await.unwrap();
    let other = WriterKey::derive(&other).await.unwrap();
    let elsewhere = WriterKey::derive(&elsewhere).await.unwrap();

    assert_eq!(first.did(), again.did(), "one (seed, session id), one key");
    assert_ne!(first.did(), other.did(), "another MCP session, another key");
    assert_ne!(
        first.did(),
        elsewhere.did(),
        "another OAuth session, another key"
    );
    // The writer acts for the agent space through its session.
    assert_eq!(first.chain().audience(), first.did());
    assert_eq!(first.chain().issuer(), &space.did());
    assert_eq!(first.chain().proof_cids().len(), 2);
}

#[dialog_common::test]
async fn a_powerline_to_another_key_is_refused() {
    let space = Ed25519Signer::generate().await.unwrap();
    let remote = url::Url::parse("http://127.0.0.1:1/ucan/").unwrap();
    let mut session = session(&space, "mcp-1", &remote).await;
    session.seed = Zeroizing::new(rand::random::<[u8; 32]>());

    let error = WriterKey::derive(&session).await.err().unwrap();

    assert!(matches!(error, TonkWorkerError::Forbidden(_)), "{error:?}");
}

#[dialog_common::test]
fn only_routes_that_cannot_commit_are_reads() {
    let read = |method: Method, uri: &str| is_read(&method, &uri.parse::<Uri>().unwrap());
    assert!(read(Method::GET, "/blob/abc"));
    assert!(read(Method::POST, "/query"));
    assert!(read(Method::POST, "/evaluate?transact=false"));
    // `transact` defaults to true.
    assert!(!read(Method::POST, "/evaluate"));
    assert!(!read(Method::POST, "/evaluate?transact=true"));
    assert!(!read(
        Method::POST,
        "/evaluate?transact=false&transact=true"
    ));
    assert!(!read(Method::POST, "/evaluate/conditional?transact=false"));
    assert!(!read(Method::POST, "/blob"));
    assert!(!read(Method::POST, "/transact"));
    assert!(!read(Method::POST, "/sync/push"));
}

#[cfg(all(feature = "integration-tests", not(target_arch = "wasm32")))]
mod against_a_service {
    //! These need the native access service, so they run natively only.
    //! In a Workers isolate the same [`SpaceWriter`] runs over in-memory
    //! storage; nothing here depends on the storage behind it.

    use std::collections::BTreeSet;

    use dialog_common::helpers::{Provisionable as _, Service};
    use futures_util::future::join_all;
    use tonk_access_service::helpers::{AccessServer, AccessServiceAddress};

    use super::*;

    const SCHEMA: &str = "attribute!: &writer-note-title
  description: Title
  the: writer.note/title
  as: text
attribute!: &writer-note-tag
  description: Tag
  the: writer.note/tag
  as: text
concept!: &writer-note
  description: A note
  with:
    title: writer-note-title
    tag: writer-note-tag
";

    /// An access service serving a fresh agent space, with the space's
    /// schema in place.
    struct Fixture {
        service: Service<AccessServiceAddress, AccessServer>,
        space: Ed25519Signer,
        remote: url::Url,
        run: u64,
    }

    impl Fixture {
        async fn start() -> Self {
            let service = AccessServiceAddress::start(Default::default())
                .await
                .unwrap();
            let space = Ed25519Signer::generate().await.unwrap();
            service
                .address
                .provision_subject(space.did().as_ref())
                .await
                .unwrap();
            let remote = url::Url::parse(&service.address.ucan_endpoint()).unwrap();
            let fixture = Self {
                service,
                space,
                remote,
                run: rand::random(),
            };
            let writer = fixture.writer("schema").await;
            let (status, body) = evaluate(&writer, SCHEMA.to_owned()).await;
            assert!(status.is_success(), "schema: {status}: {body}");
            writer.close().await;
            fixture
        }

        /// The writer of the space for a new OAuth session's MCP session
        /// `mcp_session`.
        async fn writer(&self, mcp_session: &str) -> SpaceWriter {
            let session = session(&self.space, mcp_session, &self.remote).await;
            self.writer_for(&session).await
        }

        async fn writer_for(&self, session: &WriterSession) -> SpaceWriter {
            SpaceWriter::open(&scratch_root(), session, &self.space.did(), &self.remote)
                .await
                .unwrap()
        }

        fn note(&self, label: &str) -> String {
            format!(
                "writer-note!:\n  this: urn:writer:{}:{label}\n  title: \"{label}\"\n  tag: \"{}\"\n",
                self.run, self.run
            )
        }

        /// The notes of this run a fresh writer sees.
        async fn notes(&self) -> BTreeSet<String> {
            let reader = self.writer("reader").await;
            let notes = notes(&reader, self.run).await;
            reader.close().await;
            notes
        }

        async fn stop(self) {
            self.service.stop().await.unwrap();
        }
    }

    /// A directory no other peer uses.
    fn scratch_root() -> String {
        std::env::temp_dir()
            .join(format!("space-writer-test-{}", rand::random::<u64>()))
            .to_string_lossy()
            .into_owned()
    }

    fn scratch() -> Directory {
        Directory::At(
            std::env::temp_dir()
                .join(format!("space-writer-test-{}", rand::random::<u64>()))
                .to_string_lossy()
                .into_owned(),
        )
    }

    fn request(method: Method, uri: &str, body: impl Into<Body>) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "text/plain")
            .body(body.into())
            .unwrap()
    }

    async fn json(response: Response<Body>) -> (StatusCode, serde_json::Value) {
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let value = serde_json::from_slice(&body).unwrap_or_else(|_| {
            serde_json::Value::String(String::from_utf8_lossy(&body).into_owned())
        });
        (status, value)
    }

    async fn evaluate(writer: &SpaceWriter, document: String) -> (StatusCode, serde_json::Value) {
        json(
            writer
                .write(request(Method::POST, "/evaluate?transact=true", document))
                .await,
        )
        .await
    }

    async fn query(writer: &SpaceWriter, document: &str) -> (StatusCode, serde_json::Value) {
        json(
            writer
                .read(request(
                    Method::POST,
                    "/evaluate?transact=false",
                    document.to_owned(),
                ))
                .await,
        )
        .await
    }

    /// The notes of `run` that `writer` reads.
    async fn notes(writer: &SpaceWriter, run: u64) -> BTreeSet<String> {
        let (status, body) = query(
            writer,
            &format!("writer.note:\n  this: ?n\n  tag: \"{run}\"\n"),
        )
        .await;
        assert!(status.is_success(), "query: {status}: {body}");
        entities(&body["matches_after"], &format!("urn:writer:{run}:"))
    }

    /// Every string in `value` that starts with `prefix`.
    fn entities(value: &serde_json::Value, prefix: &str) -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        let text = value.to_string();
        let mut rest = text.as_str();
        while let Some(at) = rest.find(prefix) {
            let tail = &rest[at..];
            let end = tail.find('"').unwrap_or(tail.len());
            found.insert(tail[..end].to_owned());
            rest = &tail[end..];
        }
        found
    }

    fn labels(run: u64, labels: &[String]) -> BTreeSet<String> {
        labels
            .iter()
            .map(|label| format!("urn:writer:{run}:{label}"))
            .collect()
    }

    #[dialog_common::test]
    async fn parallel_writes_through_one_session_all_land() {
        let fixture = Fixture::start().await;
        let writer = fixture.writer("mcp-1").await;
        assert_eq!(
            writer.peer().state.read().await.profile.caches().budget(),
            NODE_CACHE_BUDGET,
            "the writer's node cache is budgeted"
        );
        let written: Vec<String> = (0..5).map(|i| format!("one-session-{i}")).collect();

        let answers = join_all(
            written
                .iter()
                .map(|label| evaluate(&writer, fixture.note(label))),
        )
        .await;

        let mut editions = BTreeSet::new();
        for (status, body) in &answers {
            assert!(status.is_success(), "write: {status}: {body}");
            editions.insert(body["revision_after"].to_string());
        }
        assert_eq!(editions.len(), 5, "each write is its own revision");
        let expected = labels(fixture.run, &written);
        assert_eq!(notes(&writer, fixture.run).await, expected);
        assert_eq!(fixture.notes().await, expected, "every write landed");
        writer.close().await;
        fixture.stop().await;
    }

    #[dialog_common::test]
    async fn parallel_writes_from_two_sessions_both_land() {
        let fixture = Fixture::start().await;
        // Two MCP sessions of one OAuth session: one seed, two ids.
        let seed = rand::random::<[u8; 32]>();
        let first = session_with(
            &fixture.space,
            Zeroizing::new(seed),
            "mcp-1",
            &fixture.remote,
        )
        .await;
        let second = session_with(
            &fixture.space,
            Zeroizing::new(seed),
            "mcp-2",
            &fixture.remote,
        )
        .await;
        let first = fixture.writer_for(&first).await;
        let second = fixture.writer_for(&second).await;
        assert_ne!(first.did(), second.did());
        assert_ne!(first.operator().await, second.operator().await);
        let written: Vec<String> = (0..6).map(|i| format!("two-sessions-{i}")).collect();

        let answers = join_all(written.iter().enumerate().map(|(i, label)| {
            let writer = if i % 2 == 0 { &first } else { &second };
            evaluate(writer, fixture.note(label))
        }))
        .await;

        let mut issuers = BTreeSet::new();
        for (status, body) in &answers {
            assert!(status.is_success(), "write: {status}: {body}");
            issuers.insert(body["revision_after"]["issuer"].to_string());
        }
        assert_eq!(issuers.len(), 2, "each session issues as its own DID");
        assert_eq!(fixture.notes().await, labels(fixture.run, &written));
        first.close().await;
        second.close().await;
        fixture.stop().await;
    }

    #[dialog_common::test]
    async fn reads_never_publish_a_revision() {
        let fixture = Fixture::start().await;
        let reader = fixture.writer("mcp-reader").await;
        let other = fixture.writer("mcp-other").await;
        let (status, body) = evaluate(&reader, fixture.note("by-reader")).await;
        assert!(status.is_success(), "{status}: {body}");
        // Another session writes after it, so the reader's next pull has a
        // head to adopt.
        let (status, body) = evaluate(&other, fixture.note("by-other")).await;
        assert!(status.is_success(), "{status}: {body}");
        let head = other.status().await.unwrap().remote.unwrap();
        assert_eq!(
            serde_json::to_value(&head).unwrap(),
            body["revision_after"],
            "the other session's write is the head"
        );

        let answers = join_all((0..4).map(|_| notes(&reader, fixture.run))).await;

        for seen in answers {
            assert_eq!(
                seen,
                labels(fixture.run, &["by-reader".into(), "by-other".into()])
            );
        }
        let status = reader.status().await.unwrap();
        assert_eq!(status.remote.as_ref(), Some(&head), "nothing was pushed");
        assert_eq!(
            status.local.as_ref(),
            Some(&head),
            "the reader adopted the other session's head and minted nothing"
        );

        // A committing route sent as a read is refused, and writes nothing.
        let refused = reader
            .read(request(
                Method::POST,
                "/evaluate",
                fixture.note("refused-read"),
            ))
            .await;
        let (status, body) = json(refused).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(body["code"], "NOT_A_READ");
        let status = reader.status().await.unwrap();
        assert_eq!(status.local.as_ref(), Some(&head));
        assert_eq!(status.remote.as_ref(), Some(&head));
        assert!(
            !fixture
                .notes()
                .await
                .contains(&format!("urn:writer:{}:refused-read", fixture.run))
        );
        reader.close().await;
        other.close().await;
        fixture.stop().await;
    }

    /// A write whose push fails is answered as failed and is gone: the
    /// writer's head goes back to where it was before the write, and the
    /// next write lands without it.
    #[dialog_common::test]
    async fn a_write_that_did_not_land_never_lands_later() {
        let fixture = Fixture::start().await;
        let writer = fixture.writer("mcp-1").await;
        let (status, body) = evaluate(&writer, fixture.note("before")).await;
        assert!(status.is_success(), "{status}: {body}");
        let before = writer.status().await.unwrap();
        assert!(before.local.is_some());
        assert_eq!(before.local, before.remote);

        writer.refuse_push.store(true, Ordering::SeqCst);
        let (status, body) = evaluate(&writer, fixture.note("refused")).await;

        assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
        assert_eq!(body["code"], "WRITE_NOT_LANDED");
        let after = writer.status().await.unwrap();
        assert_eq!(
            after.local, before.local,
            "the head is back before the write"
        );
        assert_eq!(after.remote, before.remote, "nothing was published");
        let expected = labels(fixture.run, &["before".into()]);
        assert_eq!(notes(&writer, fixture.run).await, expected);

        let (status, body) = evaluate(&writer, fixture.note("after")).await;
        assert!(status.is_success(), "{status}: {body}");
        let expected = labels(fixture.run, &["before".into(), "after".into()]);
        assert_eq!(
            fixture.notes().await,
            expected,
            "the next push carried the write after, and not the refused one"
        );
        assert_eq!(notes(&writer, fixture.run).await, expected);
        writer.close().await;
        fixture.stop().await;
    }

    /// dialog-db/dialog-db#705, as the design meets it: two peers signing
    /// as one DID that commit on the same head both mint the same version,
    /// both pushes succeed, and one write is gone. One writer per MCP
    /// session, with its own DID, is what keeps this from happening.
    ///
    /// When dialog detects the duplicate version, the second push fails
    /// instead and this test should be turned around to assert that.
    #[dialog_common::test]
    async fn writers_sharing_a_did_lose_a_write() {
        let fixture = Fixture::start().await;
        let session = session(&fixture.space, "mcp-shared", &fixture.remote).await;
        let key = WriterKey::derive(&session).await.unwrap();
        let open = || async {
            let peer = HostedPeer::open_in(
                scratch(),
                &key.seed,
                std::slice::from_ref(key.chain()),
                HeldCaches::new(),
            )
            .await
            .unwrap();
            peer.mount(&fixture.space.did(), &fixture.remote)
                .await
                .unwrap();
            peer
        };
        let first = open().await;
        let second = open().await;
        let base = format!("/api/repository/{}/branch/main", fixture.space.did());
        async fn call(peer: &HostedPeer, path: String, body: String) -> Response<Body> {
            peer.call(request(Method::POST, &path, body)).await
        }
        // Both on the same head.
        for peer in [&first, &second] {
            let (status, body) =
                json(call(peer, format!("{base}/sync/pull"), String::new()).await).await;
            assert!(status.is_success(), "pull: {status}: {body}");
        }

        let (status, a) = json(
            call(
                &first,
                format!("{base}/evaluate?transact=true"),
                fixture.note("shared-a"),
            )
            .await,
        )
        .await;
        assert!(status.is_success(), "{status}: {a}");
        let (status, body) =
            json(call(&first, format!("{base}/sync/push"), String::new()).await).await;
        assert!(status.is_success(), "first push: {status}: {body}");
        let (status, b) = json(
            call(
                &second,
                format!("{base}/evaluate?transact=true"),
                fixture.note("shared-b"),
            )
            .await,
        )
        .await;
        assert!(status.is_success(), "{status}: {b}");
        let (status, body) =
            json(call(&second, format!("{base}/sync/push"), String::new()).await).await;
        assert!(status.is_success(), "second push: {status}: {body}");

        assert_eq!(
            a["revision_after"]["issuer"], b["revision_after"]["issuer"],
            "one DID"
        );
        assert_eq!(
            a["revision_after"]["edition"], b["revision_after"]["edition"],
            "one version, minted twice"
        );
        let landed = fixture.notes().await;
        assert_eq!(landed.len(), 1, "one of the two writes is lost: {landed:?}");
        first.close().await;
        second.close().await;
        fixture.stop().await;
    }

    /// `space -> device -> agent` for `verb`, one chain per storage
    /// scope: what the approval retains in the agent space.
    async fn grant(space: &Ed25519Signer, agent: &Did, verb: &str) -> Vec<DelegationChain> {
        use dialog_credentials::Signer;
        use dialog_ucan_core::DelegationBuilder;
        use dialog_ucan_core::subject::Subject as UcanSubject;
        let device = Ed25519Signer::generate().await.unwrap();
        let mut chains = Vec::new();
        for (kind, item) in [
            ("memory", "cell"),
            ("archive", "block"),
            ("archive", "blob"),
        ] {
            let command: Vec<String> = vec!["use".into(), verb.into(), kind.into(), item.into()];
            let mut links = Vec::new();
            for (issuer, audience) in [(space, &device.did()), (&device, agent)] {
                links.push(
                    DelegationBuilder::new()
                        .issuer(Signer::from(issuer.clone()))
                        .audience(audience)
                        .subject(UcanSubject::Specific(space.did()))
                        .command(command.clone())
                        .try_build()
                        .await
                        .unwrap(),
                );
            }
            chains.push(DelegationChain::try_from(links).unwrap());
        }
        chains
    }

    /// The writer of another space finds its grant in the agent space, as
    /// a device finds what its account holds, and writes with it; the
    /// agent space's writer classifies what it holds; a space with no
    /// grant there is refused by the access service at its writer's
    /// first pull, and nothing lands.
    #[dialog_common::test]
    async fn a_grant_in_the_agent_space_lets_a_writer_act_on_its_space() {
        let fixture = Fixture::start().await;
        let agent = fixture.space.did();
        let granted = Ed25519Signer::generate().await.unwrap();
        let ungranted = Ed25519Signer::generate().await.unwrap();
        for space in [&granted, &ungranted] {
            fixture
                .service
                .address
                .provision_subject(space.did().as_ref())
                .await
                .unwrap();
        }
        let mut chains = grant(&granted, &agent, "get").await;
        chains.extend(grant(&granted, &agent, "put").await);
        let approval = fixture.writer("approval").await;
        approval
            .approve(
                &chains,
                &[super::super::super::access::DirectoryEntry {
                    space: granted.did(),
                    name: Some("Garden".into()),
                    address: Some(fixture.remote.to_string()),
                }],
            )
            .await
            .unwrap();
        approval.close().await;

        let session = session(&fixture.space, "mcp-1", &fixture.remote).await;
        let own = fixture.writer_for(&session).await;
        let access = own.agent_access().await.unwrap();
        let entry = access.get(&granted.did()).expect("the granted space");
        assert!(entry.read && entry.write, "{entry:?}");
        assert_eq!(entry.name.as_deref(), Some("Garden"));
        assert_eq!(
            entry.address.as_deref(),
            Some(fixture.remote.as_str()),
            "the directory names where it syncs"
        );
        assert_eq!(entry.grants.len(), 6, "one leaf per scope and verb");
        assert!(access.get(&agent).is_some_and(|own| own.agent));
        assert!(access.get(&ungranted.did()).is_none());

        let writer = SpaceWriter::open(&scratch_root(), &session, &granted.did(), &fixture.remote)
            .await
            .unwrap();
        let error = writer.agent_access().await.err().unwrap();
        assert!(matches!(error, TonkWorkerError::Forbidden(_)), "{error:?}");
        let (status, body) = evaluate(&writer, SCHEMA.to_owned()).await;
        assert!(status.is_success(), "schema: {status}: {body}");
        let (status, body) = evaluate(&writer, fixture.note("granted")).await;
        assert!(status.is_success(), "write: {status}: {body}");
        assert_eq!(
            notes(&writer, fixture.run).await,
            labels(fixture.run, &["granted".into()])
        );

        let refused =
            SpaceWriter::open(&scratch_root(), &session, &ungranted.did(), &fixture.remote)
                .await
                .unwrap();
        let (status, body) = evaluate(&refused, fixture.note("refused")).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
        assert_eq!(body["code"], "PULL_FAILED", "{body}");
        // Its owner sees nothing published.
        let owner = super::session(&ungranted, "owner", &fixture.remote).await;
        let owner = SpaceWriter::open(&scratch_root(), &owner, &ungranted.did(), &fixture.remote)
            .await
            .unwrap();
        assert_eq!(owner.status().await.unwrap().remote, None, "nothing landed");
        owner.close().await;
        own.close().await;
        writer.close().await;
        refused.close().await;
        fixture.stop().await;
    }
}
