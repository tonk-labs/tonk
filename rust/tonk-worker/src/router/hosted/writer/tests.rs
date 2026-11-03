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

#[dialog_common::test]
fn a_page_is_taken_off_the_route() {
    let take = |uri: &str| caps::Page::take(&uri.parse::<Uri>().unwrap());
    let (page, uri) = take("/evaluate?transact=false&limit=3&cursor=6.abc").unwrap();
    assert_eq!(uri, "/evaluate?transact=false");
    assert_ne!(page, caps::Page::default());
    let (page, uri) = take("/evaluate?transact=false").unwrap();
    assert_eq!(uri, "/evaluate?transact=false");
    assert_eq!(page, caps::Page::default());
    let (_, uri) = take("/query?limit=2").unwrap();
    assert_eq!(uri, "/query");
    assert!(take("/query?limit=0").is_err());
    assert!(take("/query?limit=many").is_err());
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
    async fn an_outline_reads_the_views_of_the_space_as_it_is_now() {
        let fixture = Fixture::start().await;
        let writer = fixture.writer("mcp-outline").await;
        let (status, body) = evaluate(
            &writer,
            "view!:\n  this: writer-note\n  show:\n    directory: \"<ul><li>{title}</li></ul>\"\n"
                .to_owned(),
        )
        .await;
        assert!(status.is_success(), "view: {status}: {body}");
        for label in ["first", "second"] {
            let (status, body) = evaluate(&writer, fixture.note(label)).await;
            assert!(status.is_success(), "note: {status}: {body}");
        }
        // A reader in another session sees the writes, as an agent's next
        // call would.
        let reader = fixture.writer("mcp-outline-reader").await;
        let outline = reader.outline("writer-note").await.unwrap();
        assert_eq!(outline["space"], fixture.space.did().to_string());
        assert_eq!(outline["outline"]["model"], "writer-note", "{outline}");
        assert_eq!(outline["outline"]["route"], "writer-note");
        assert!(
            outline["outline"]["records"].as_u64().unwrap() >= 2,
            "{outline}"
        );
        assert!(
            reader.outline("@writer-note").await.is_err(),
            "a malformed route is refused"
        );
        reader.close().await;
        writer.close().await;
        fixture.stop().await;
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

    /// A write holds the writer from its pull through its push, so the
    /// Host Worker never recycles a writer with a write in it.
    #[dialog_common::test]
    async fn a_write_in_flight_holds_the_writer() {
        let fixture = Fixture::start().await;
        let writer = fixture.writer("mcp-1").await;
        assert!(!writer.holds_write(), "an idle writer holds no write");

        let (status, body) = {
            let mut write = std::pin::pin!(evaluate(&writer, fixture.note("held")));
            assert!(
                futures_util::poll!(write.as_mut()).is_pending(),
                "the write waits on its pull"
            );
            assert!(writer.holds_write(), "a write in flight holds the writer");
            write.await
        };
        assert!(status.is_success(), "{status}: {body}");
        assert!(!writer.holds_write(), "an answered write lets go");
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
    /// scope, the leaf carrying `home` in its signed meta: what the
    /// approval retains in the agent space.
    async fn grant(
        space: &Ed25519Signer,
        agent: &Did,
        verb: &str,
        home: &url::Url,
    ) -> Vec<DelegationChain> {
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
            links.push(
                DelegationBuilder::new()
                    .issuer(Signer::from(space.clone()))
                    .audience(&device.did())
                    .subject(UcanSubject::Specific(space.did()))
                    .command(command.clone())
                    .try_build()
                    .await
                    .unwrap(),
            );
            links.push(
                DelegationBuilder::new()
                    .issuer(Signer::from(device.clone()))
                    .audience(agent)
                    .subject(UcanSubject::Specific(space.did()))
                    .command(command.clone())
                    .meta(tonk_invite::home_address_meta(home))
                    .try_build()
                    .await
                    .unwrap(),
            );
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
        let mut chains = grant(&granted, &agent, "get", &fixture.remote).await;
        chains.extend(grant(&granted, &agent, "put", &fixture.remote).await);
        let approval = fixture.writer("approval").await;
        approval
            .approve(
                &chains,
                &[super::super::super::access::DirectoryEntry {
                    space: granted.did(),
                    name: Some("Garden".into()),
                    address: Some("https://elsewhere.example/ucan/".into()),
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
            "the grant, not the directory, names where it syncs"
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

    /// Write the notes `labels` through `writer`, one write each.
    async fn write_notes(fixture: &Fixture, writer: &SpaceWriter, labels: &[String]) {
        for label in labels {
            let (status, body) = evaluate(writer, fixture.note(label)).await;
            assert!(status.is_success(), "write: {status}: {body}");
        }
    }

    fn notes_of(run: u64) -> String {
        format!("writer.note:\n  this: ?n\n  tag: \"{run}\"\n")
    }

    async fn page(
        writer: &SpaceWriter,
        document: &str,
        limit: usize,
        cursor: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let cursor = cursor.map(|cursor| format!("&cursor={cursor}"));
        let uri = format!(
            "/evaluate?transact=false&limit={limit}{}",
            cursor.unwrap_or_default()
        );
        json(
            writer
                .read(request(Method::POST, &uri, document.to_owned()))
                .await,
        )
        .await
    }

    #[dialog_common::test]
    async fn a_read_over_the_cap_is_refused() {
        let fixture = Fixture::start().await;
        let writer = fixture.writer("mcp-1").await;
        let written: Vec<String> = (0..4).map(|i| format!("capped-{i}")).collect();
        write_notes(&fixture, &writer, &written).await;
        let document = notes_of(fixture.run);

        let rows = writer.with_read_caps(ReadCaps {
            rows: 3,
            bytes: caps::MAX_READ_BYTES,
        });
        let (status, body) = query(&rows, &document).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
        assert_eq!(body["code"], "READ_TOO_LARGE");
        assert!(
            body["error"].as_str().unwrap().contains("4 rows"),
            "the refusal says how big the read was: {body}"
        );
        // Asked for more than the cap in one page, too.
        let (status, body) = page(&rows, &document, 4, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["code"], "LIMIT_TOO_LARGE");
        rows.close().await;

        let writer = fixture.writer("mcp-2").await.with_read_caps(ReadCaps {
            rows: caps::MAX_READ_ROWS,
            bytes: 256,
        });
        let (status, body) = query(&writer, &document).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
        assert_eq!(body["code"], "READ_TOO_LARGE");
        writer.close().await;

        // Under the caps, a read is answered as the branch answers it.
        let writer = fixture.writer("mcp-3").await;
        assert_eq!(
            notes(&writer, fixture.run).await,
            labels(fixture.run, &written)
        );
        writer.close().await;
        fixture.stop().await;
    }

    #[dialog_common::test]
    async fn a_paged_read_returns_every_row_and_publishes_nothing() {
        let fixture = Fixture::start().await;
        let writer = fixture.writer("mcp-1").await;
        let written: Vec<String> = (0..7).map(|i| format!("paged-{i}")).collect();
        write_notes(&fixture, &writer, &written).await;
        writer.close().await;
        let reader = fixture.writer("mcp-reader").await.with_read_caps(ReadCaps {
            rows: 3,
            bytes: caps::MAX_READ_BYTES,
        });
        let document = notes_of(fixture.run);
        // The reader's first read pulls the space's head.
        let (status, body) = page(&reader, &document, 1, None).await;
        assert!(status.is_success(), "page: {status}: {body}");
        let before = reader.status().await.unwrap();
        assert!(before.local.is_some());
        assert_eq!(before.local, before.remote);

        let mut seen = BTreeSet::new();
        let mut pages = 0;
        let mut cursor: Option<String> = None;
        loop {
            let (status, body) = page(&reader, &document, 3, cursor.as_deref()).await;
            assert!(status.is_success(), "page: {status}: {body}");
            assert_eq!(body["page"]["rows"], 7);
            assert_eq!(body["matches_before"], body["matches_after"]);
            let rows = entities(
                &body["matches_before"],
                &format!("urn:writer:{}:", fixture.run),
            );
            assert!(rows.len() <= 3, "a page holds at most `limit` rows");
            assert!(rows.is_disjoint(&seen), "no row is answered twice");
            seen.extend(rows);
            pages += 1;
            match body["page"]["next"].as_str() {
                Some(next) => cursor = Some(next.to_owned()),
                None => break,
            }
        }
        assert_eq!(pages, 3);
        assert_eq!(seen, labels(fixture.run, &written), "every row, once");

        let after = reader.status().await.unwrap();
        assert_eq!(after.local, before.local, "no read minted a revision");
        assert_eq!(after.remote, before.remote, "no read published one");

        // A cursor from before the space changed is refused, not followed.
        let (_, first) = page(&reader, &document, 3, None).await;
        let stale = first["page"]["next"].as_str().unwrap().to_owned();
        let other = fixture.writer("mcp-other").await;
        write_notes(&fixture, &other, &["paged-late".into()]).await;
        let (status, body) = page(&reader, &document, 3, Some(&stale)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["code"], "PAGE_STALE");
        let (status, body) = page(&reader, &document, 3, Some("nonsense")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["code"], "BAD_CURSOR");

        reader.close().await;
        other.close().await;
        fixture.stop().await;
    }

    /// Schema discovery, the first read an agent makes, pages like any other.
    #[dialog_common::test]
    async fn a_concept_read_pages() {
        let fixture = Fixture::start().await;
        let reader = fixture.writer("mcp-reader").await;
        let (status, whole) = query(&reader, "concept:\n").await;
        assert!(status.is_success(), "{status}: {whole}");
        let total: usize = whole["matches_before"]
            .as_array()
            .unwrap()
            .iter()
            .map(|block| block["results"].as_array().unwrap().len())
            .sum();
        assert!(total > 1, "the space has concepts: {whole}");

        let mut seen = 0;
        let mut cursor: Option<String> = None;
        loop {
            let (status, body) = page(&reader, "concept:\n", 1, cursor.as_deref()).await;
            assert!(status.is_success(), "{status}: {body}");
            assert_eq!(body["page"]["rows"], total);
            let rows: usize = body["matches_before"]
                .as_array()
                .unwrap()
                .iter()
                .map(|block| block["results"].as_array().unwrap().len())
                .sum();
            assert_eq!(rows, 1, "a page of one concept: {body}");
            seen += rows;
            match body["page"]["next"].as_str() {
                Some(next) => cursor = Some(next.to_owned()),
                None => break,
            }
        }
        assert_eq!(seen, total, "every concept, one page each");
        reader.close().await;
        fixture.stop().await;
    }

    /// A member's share link to `space`, as `tonk_join_space` reads one:
    /// `space -> link key` for every command, syncing at `home`, the key's
    /// seed beside it.
    async fn share_link(space: &Ed25519Signer, home: &url::Url) -> tonk_invite::Invite {
        use dialog_credentials::Signer;
        use dialog_ucan_core::DelegationBuilder;
        use dialog_ucan_core::subject::Subject as UcanSubject;
        let seed: [u8; 32] = rand::random();
        let key = Ed25519Signer::import(&seed).await.unwrap();
        let root = DelegationBuilder::new()
            .issuer(Signer::from(space.clone()))
            .audience(&key.did())
            .subject(UcanSubject::Specific(space.did()))
            .command(vec![])
            .meta(tonk_invite::home_address_meta(home))
            .try_build()
            .await
            .unwrap();
        tonk_invite::Invite::new(
            DelegationChain::new(root),
            tonk_invite::InviteAudience::Open { seed },
            Some(home.clone()),
        )
        .await
        .unwrap()
    }

    /// The browser's authority over its agent space does not extend to an
    /// external space granted to that agent. Contrast its rejected revocation
    /// with the hosted session's successful withdrawal of the same grant.
    #[dialog_common::test]
    async fn browser_cannot_revoke_link_issued_agent_grants_but_session_can() {
        use crate::router::{agent_connections::issue_to_recipient, repository};
        use dialog_capability::{
            Subject,
            access::{Access, Prove},
        };
        use dialog_credentials::Signer;
        use dialog_ucan::{Parameters, Scope, Ucan};
        use dialog_ucan_core::{
            command::Command, subject::Subject as UcanSubject, time::Timestamp,
        };
        use tonk_invite::connection::{AgentAccess, agent_grant_scopes};

        let fixture = Fixture::start().await;
        let browser = crate::router::command::tests::native::test_state().await;
        let (agent, agent_session, chains) = {
            let root = Ed25519Signer::generate().await.unwrap();
            {
                let tonk = browser.read().await;
                let device_grant = tonk_identity::delegation::mint_device_delegation(
                    root.clone(),
                    &tonk.profile.did(),
                )
                .await
                .unwrap();
                crate::router::identity::persist_root(
                    &tonk,
                    tonk_worker_api::SaveRootRequest {
                        credential_id: "spike-no-passkey".into(),
                        delegation_hex: hex::encode(device_grant.to_bytes().unwrap()),
                        passkey: None,
                        encryption_key: None,
                    },
                )
                .await
                .unwrap();
            }
            let seed = Zeroizing::new(rand::random::<[u8; 32]>());
            let agent_signer = Ed25519Signer::import(&*seed).await.unwrap();
            let agent = agent_signer.did();
            repository::create_space_from_seed(&browser, "Revocation spike agent", None, seed)
                .await
                .unwrap();
            fixture
                .service
                .address
                .provision_subject(agent.as_ref())
                .await
                .unwrap();
            let agent_session =
                session(&agent_signer, "recipient-revocation", &fixture.remote).await;

            let invite = share_link(&fixture.space, &fixture.remote).await;
            let link_signer = invite.signer().await.unwrap().unwrap();
            let scopes = agent_grant_scopes(&fixture.space.did(), AgentAccess::Read);
            let now = Timestamp::now();
            let expires = Timestamp::try_from(i128::from(now.to_unix() + 3600)).unwrap();
            let chains = issue_to_recipient(
                &agent,
                Signer::from(link_signer),
                vec![invite.chain.clone(); scopes.len()],
                &scopes,
                &fixture.remote,
                now,
                expires,
            )
            .await
            .unwrap()
            .chains()
            .to_vec();
            (agent, agent_session, chains)
        };
        // No account root, agent key, link key, or passkey is available below.
        let own = SpaceWriter::open(&scratch_root(), &agent_session, &agent, &fixture.remote)
            .await
            .unwrap();
        own.approve(
            &chains,
            &[super::super::super::access::DirectoryEntry {
                space: fixture.space.did(),
                name: Some("Target".into()),
                address: Some(fixture.remote.to_string()),
            }],
        )
        .await
        .unwrap();

        let reader = SpaceWriter::open(
            &scratch_root(),
            &agent_session,
            &fixture.space.did(),
            &fixture.remote,
        )
        .await
        .unwrap();
        let (status, body) = query(&reader, "concept:\n").await;
        assert!(
            status.is_success(),
            "agent read before revocation: {status}: {body}"
        );

        let (device, authority) = {
            let tonk = browser.read().await;
            assert!(
                !crate::router::join::find_replica_for_subject(&tonk, &fixture.space.did())
                    .await
                    .unwrap(),
                "the browser must not join the target space"
            );
            let scope = Scope {
                subject: UcanSubject::Specific(agent.clone()),
                command: Command(vec![]),
                parameters: Parameters::default(),
            };
            let proof = Subject::from(tonk.profile.did())
                .attenuate(Access)
                .invoke(Prove::<Ucan>::new(tonk.profile.did(), scope))
                .perform(&tonk.operator)
                .await
                .unwrap();
            let mut certificates = proof.proofs.into_iter().map(|proof| proof.0);
            let mut authority = DelegationChain::new(certificates.next().unwrap());
            for certificate in certificates {
                authority = authority.push(certificate).unwrap();
            }
            assert_eq!(authority.issuer(), &agent);
            assert_eq!(authority.audience(), &tonk.profile.did());
            assert_eq!(authority.proofs().count(), 2, "agent -> account -> device");
            assert_eq!(
                authority.proofs().next().unwrap().subject(),
                &UcanSubject::Specific(agent.clone()),
                "the browser controls the agent's own space, not arbitrary external capabilities"
            );
            (tonk.profile.credential().signer().clone(), authority)
        };
        for chain in &chains {
            let target = *chain.proof_cids().last().unwrap();
            let artifact = tonk_identity::revocation::mint_delegated_revocation_with_witness(
                device.clone(),
                chain,
                &target,
                &authority,
            )
            .await
            .unwrap();
            assert!(
                matches!(
                    tonk_identity::revocation::verify(&artifact).await,
                    Err(tonk_identity::revocation::VerifyError::Unauthorized(_))
                ),
                "the scoped agent-space grant does not witness possession of target-space authority"
            );
            let refused = crate::router::http::post_cbor(&fixture.remote, &artifact)
                .await
                .expect_err("the service must reject the browser's revocation");
            assert!(
                matches!(refused, crate::router::http::HttpError::Upstream(ref error)
                if matches!(error.status, 401 | 403) && error.message.contains("witness path")),
                "{refused}"
            );
        }
        let (status, body) = query(&reader, "concept:\n").await;
        assert!(
            status.is_success(),
            "a refused browser revocation leaves the agent able to read: {status}: {body}"
        );

        // Positive control: the hosted session holds an actual powerline from
        // the agent. This secret belongs to the hosted client, not the browser.
        let session_signer = Ed25519Signer::import(&*agent_session.seed).await.unwrap();
        for chain in &chains {
            let target = *chain.proof_cids().last().unwrap();
            let artifact = tonk_identity::revocation::mint_delegated_revocation_with_witness(
                session_signer.clone(),
                chain,
                &target,
                &agent_session.powerline,
            )
            .await
            .unwrap();
            let verified = tonk_identity::revocation::verify(&artifact).await.unwrap();
            assert_eq!(verified.issuer, session_signer.did());
            assert_eq!(verified.subject, agent);
            let response = crate::router::http::post_cbor(&fixture.remote, &artifact)
                .await
                .unwrap();
            let receipt: tonk_account::customer::RevokeReceipt =
                serde_json::from_slice(&response.body).unwrap();
            assert_eq!(receipt.revoked, target);
        }
        // Keep both the writer's cached proofs and the published grants intact.
        let (status, body) = query(&reader, "concept:\n").await;
        assert!(
            !status.is_success(),
            "same writer read after revocation: {body}"
        );
        assert!(
            body.to_string().contains("CREDENTIAL_REVOKED"),
            "{status}: {body}"
        );
        assert!(
            own.agent_access()
                .await
                .unwrap()
                .get(&fixture.space.did())
                .unwrap()
                .read,
            "published grants are still present: revocation, not deletion, blocks the read"
        );

        reader.close().await;
        own.close().await;
        fixture.stop().await;
    }

    /// The person removes access a share link gave their agent, with the
    /// passkey: the account opens the agent's sealed key and the agent
    /// revokes, as the grants' audience, every leaf the link issued to
    /// it. The agent joined straight from the link; the person never
    /// joined the space. The access service then refuses the grants
    /// wherever they are presented, while the hosted copy still lists
    /// them: revocation, not deletion, is what refuses them.
    #[dialog_common::test]
    async fn the_account_withdraws_a_link_grant_as_its_agent() {
        use crate::router::{
            agent_connections::issue_to_recipient,
            agent_session::{Granted, mark, record_grants},
            agents::withdraw,
            repository,
        };
        use dialog_credentials::Signer;
        use dialog_ucan_core::time::Timestamp;
        use tonk_invite::connection::{AgentAccess, agent_grant_scopes};
        use tonk_schema::prelude::DidExt as _;

        let fixture = Fixture::start().await;
        let space = Ed25519Signer::generate().await.unwrap();
        fixture
            .service
            .address
            .provision_subject(space.did().as_ref())
            .await
            .unwrap();
        {
            let owner = super::session(&space, "owner", &fixture.remote).await;
            let owner = SpaceWriter::open(&scratch_root(), &owner, &space.did(), &fixture.remote)
                .await
                .unwrap();
            let (status, body) = evaluate(&owner, SCHEMA.to_owned()).await;
            assert!(status.is_success(), "schema: {status}: {body}");
            let (status, body) = evaluate(&owner, fixture.note("owner")).await;
            assert!(status.is_success(), "owner: {status}: {body}");
            owner.close().await;
        }

        // A browser signed in to an account, which makes the agent the
        // way the approval page does: its key seals to the account.
        let state = crate::router::command::tests::native::test_state().await;
        let root = Ed25519Signer::generate().await.unwrap();
        {
            let tonk = state.read().await;
            let device = tonk_identity::delegation::mint_device_delegation(
                root.clone(),
                &tonk.profile.did(),
            )
            .await
            .unwrap();
            crate::router::identity::persist_root(
                &tonk,
                tonk_worker_api::SaveRootRequest {
                    credential_id: "withdraw-test".into(),
                    delegation_hex: hex::encode(device.to_bytes().unwrap()),
                    passkey: None,
                    encryption_key: None,
                },
            )
            .await
            .unwrap();
            let union = tonk_account::delegations::mint_account_union(
                &Signer::from(root.clone()),
                &tonk.profile.did(),
            )
            .await
            .unwrap();
            tonk_account::peer::hand_over(&tonk.profile, &union)
                .await
                .unwrap();
        }
        let seed = Zeroizing::new(rand::random::<[u8; 32]>());
        let agent_signer = Ed25519Signer::import(&*seed).await.unwrap();
        let agent = agent_signer.did();
        let key = repository::create_space_from_seed(&state, "Claude", None, seed)
            .await
            .unwrap();
        mark(&*state.read().await, &key, &agent, Some("claude"))
            .await
            .unwrap();
        fixture
            .service
            .address
            .provision_subject(agent.as_ref())
            .await
            .unwrap();
        let agent_session = session(&agent_signer, "recipient", &fixture.remote).await;

        // The link's key grants read and write straight to the agent, as
        // the chat card's join does, and grants read again, as a second
        // join from the same link does: every leaf is live.
        let invite = share_link(&space, &fixture.remote).await;
        let link_key = Signer::from(invite.signer().await.unwrap().unwrap());
        let mut chains = Vec::new();
        for access in [AgentAccess::ReadWrite, AgentAccess::Read] {
            let scopes = agent_grant_scopes(&space.did(), access);
            let now = Timestamp::now();
            let expires = Timestamp::try_from(i128::from(now.to_unix() + 3_600)).unwrap();
            chains.extend(
                issue_to_recipient(
                    &agent,
                    link_key.clone(),
                    vec![invite.chain.clone(); scopes.len()],
                    &scopes,
                    &fixture.remote,
                    now,
                    expires,
                )
                .await
                .unwrap()
                .chains()
                .to_vec(),
            );
        }
        assert!(
            chains
                .iter()
                .any(|chain| chain.ability().starts_with("/use/put"))
                && chains
                    .iter()
                    .any(|chain| chain.ability().starts_with("/use/get")),
            "read and write leaves"
        );
        let own = SpaceWriter::open(&scratch_root(), &agent_session, &agent, &fixture.remote)
            .await
            .unwrap();
        own.approve(
            &chains,
            &[super::super::super::access::DirectoryEntry {
                space: space.did(),
                name: Some("Shared".into()),
                address: Some(fixture.remote.to_string()),
            }],
        )
        .await
        .unwrap();
        // The browser's copy of the agent space, as it pulls it: here the
        // directory names an address that does not answer, to check a
        // failed publication first.
        let dead = url::Url::parse("http://127.0.0.1:9/ucan/").unwrap();
        record_grants(
            &*state.read().await,
            &key,
            &[Granted {
                subject: space.did(),
                name: "Shared".into(),
                home: dead.clone(),
                chains: chains.clone(),
            }],
        )
        .await
        .unwrap();
        {
            let tonk = state.read().await;
            assert!(
                !crate::router::join::find_replica_for_subject(&tonk, &space.did())
                    .await
                    .unwrap(),
                "the person never joins the space"
            );
            assert_eq!(
                withdraw::link_grants(&tonk, &agent, &space.did())
                    .await
                    .unwrap()
                    .len(),
                chains.len(),
                "every leaf is listed for removal"
            );
        }

        let reader = SpaceWriter::open(
            &scratch_root(),
            &agent_session,
            &space.did(),
            &fixture.remote,
        )
        .await
        .unwrap();
        assert_eq!(
            notes(&reader, fixture.run).await,
            labels(fixture.run, &["owner".into()]),
            "the agent reads the space"
        );
        let (status, body) = evaluate(&reader, fixture.note("agent")).await;
        assert!(status.is_success(), "the agent writes: {status}: {body}");

        // Another account's passkey opens nothing and revokes nothing.
        let other = Ed25519Signer::generate().await.unwrap();
        let refused = withdraw::withdraw(&*state.read().await, &other, &key, &space.did())
            .await
            .unwrap_err();
        assert!(refused.contains("different account"), "{refused}");

        // A publication that fails leaves every grant listed to retry.
        let failed = withdraw::withdraw(&*state.read().await, &root, &key, &space.did())
            .await
            .unwrap();
        assert!(failed.revoked.is_empty(), "{failed:?}");
        assert_eq!(failed.unresolved.len(), chains.len());
        assert!(failed.finished().is_err());
        {
            let tonk = state.read().await;
            assert_eq!(
                withdraw::link_grants(&tonk, &agent, &space.did())
                    .await
                    .unwrap()
                    .len(),
                chains.len(),
                "nothing unacknowledged is dropped"
            );
        }
        let (status, body) = query(&reader, "concept:\n").await;
        assert!(
            status.is_success(),
            "after a refused attempt the agent still reads: {status}: {body}"
        );

        // Retry once the directory names the space's access service.
        {
            let tonk = state.read().await;
            tonk.reactor
                .repository(agent.repo_key())
                .branch(crate::router::repository::CONTENT_BRANCH)
                .transaction()
                .retract(tonk_schema::SpaceHomeAddress::new(
                    &space.did(),
                    dead.as_str(),
                ))
                .assert(tonk_schema::SpaceHomeAddress::new(
                    &space.did(),
                    fixture.remote.as_str(),
                ))
                .commit()
                .perform(&tonk.operator)
                .await
                .unwrap();
        }
        let done = withdraw::withdraw(&*state.read().await, &root, &key, &space.did())
            .await
            .unwrap();
        assert_eq!(done.revoked.len(), chains.len(), "{done:?}");
        assert_eq!(done.finished().unwrap(), chains.len());
        {
            let tonk = state.read().await;
            assert!(
                withdraw::link_grants(&tonk, &agent, &space.did())
                    .await
                    .unwrap()
                    .is_empty(),
                "acknowledged grants are dropped from the agent space"
            );
            assert!(
                tonk.sync_queue.is_dirty(agent.repo_key()),
                "the agent space is queued to sync"
            );
        }

        // The writer already open, with its proofs in hand, is refused.
        let (status, body) = query(&reader, "concept:\n").await;
        assert!(!status.is_success(), "same writer reads after: {body}");
        assert!(
            body.to_string().contains("CREDENTIAL_REVOKED"),
            "{status}: {body}"
        );
        let (status, body) = evaluate(&reader, fixture.note("late")).await;
        assert!(!status.is_success(), "same writer writes after: {body}");
        // The hosted copy still holds every grant: deletion is not why.
        let held = own.agent_access().await.unwrap();
        let held = held.get(&space.did()).expect("published grants remain");
        assert!(held.read && held.write, "{held:?}");
        // A fresh session of the agent, presenting the same grants.
        let fresh = session(&agent_signer, "fresh", &fixture.remote).await;
        let fresh = SpaceWriter::open(&scratch_root(), &fresh, &space.did(), &fixture.remote)
            .await
            .unwrap();
        let (status, body) = query(&fresh, "concept:\n").await;
        assert!(!status.is_success(), "a fresh reader is refused: {body}");
        assert!(
            body.to_string().contains("CREDENTIAL_REVOKED"),
            "{status}: {body}"
        );

        fresh.close().await;
        reader.close().await;
        own.close().await;
        fixture.stop().await;
    }

    /// The chain a join from the chat leaves: the person's account
    /// claims the link (`link key -> account`) and signs the agent's
    /// grant (`account -> agent space`). The agent reads with it and the
    /// access service refuses its write. When the account revokes the
    /// grant at the access service, the agent's next read is refused too.
    #[dialog_common::test]
    async fn the_account_revokes_an_agent_it_let_in() {
        use crate::router::agent_connections::issue_to_recipient;
        use dialog_credentials::Signer;
        use dialog_ucan_core::DelegationBuilder;
        use dialog_ucan_core::subject::Subject as UcanSubject;
        use dialog_ucan_core::time::Timestamp;
        use tonk_invite::connection::{AgentAccess, agent_grant_scopes};

        let fixture = Fixture::start().await;
        let space = Ed25519Signer::generate().await.unwrap();
        fixture
            .service
            .address
            .provision_subject(space.did().as_ref())
            .await
            .unwrap();
        let owner = super::session(&space, "owner", &fixture.remote).await;
        let owner = SpaceWriter::open(&scratch_root(), &owner, &space.did(), &fixture.remote)
            .await
            .unwrap();
        let (status, body) = evaluate(&owner, SCHEMA.to_owned()).await;
        assert!(status.is_success(), "schema: {status}: {body}");
        let (status, body) = evaluate(&owner, fixture.note("owner")).await;
        assert!(status.is_success(), "owner: {status}: {body}");
        owner.close().await;

        // The account claims the link, as a join does.
        let account = Ed25519Signer::generate().await.unwrap();
        let link = share_link(&space, &fixture.remote).await;
        let link_key = link.signer().await.unwrap().unwrap();
        let claim = DelegationBuilder::new()
            .issuer(Signer::from(link_key))
            .audience(&account.did())
            .subject(UcanSubject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let held = link.chain.clone().push(claim).unwrap();

        // The account lets the agent read.
        let agent = fixture.space.did();
        let scopes = agent_grant_scopes(&space.did(), AgentAccess::Read);
        let now = Timestamp::now();
        let expires = Timestamp::try_from(i128::from(now.to_unix() + 3_600)).unwrap();
        let chains = issue_to_recipient(
            &agent,
            Signer::from(account.clone()),
            vec![held.clone(); scopes.len()],
            &scopes,
            &fixture.remote,
            now,
            expires,
        )
        .await
        .unwrap()
        .chains()
        .to_vec();
        let session = session(&fixture.space, "mcp-join", &fixture.remote).await;
        let own = fixture.writer_for(&session).await;
        own.approve(
            &chains,
            &[super::super::super::access::DirectoryEntry {
                space: space.did(),
                name: Some("Shared".into()),
                address: Some(fixture.remote.to_string()),
            }],
        )
        .await
        .unwrap();
        let entry = own.agent_access().await.unwrap();
        let entry = entry.get(&space.did()).expect("the grant").clone();
        assert!(entry.read && !entry.write, "{entry:?}");
        own.close().await;

        let reader = SpaceWriter::open(&scratch_root(), &session, &space.did(), &fixture.remote)
            .await
            .unwrap();
        assert_eq!(
            notes(&reader, fixture.run).await,
            labels(fixture.run, &["owner".into()]),
            "the agent reads the space"
        );
        let (status, body) = evaluate(&reader, fixture.note("agent")).await;
        assert!(!status.is_success(), "a read grant cannot write: {body}");
        reader.close().await;

        // The account withdraws each leaf it signed, witnessed by its own
        // path to the space, at the space's access service.
        for chain in &chains {
            let target = *chain.proof_cids().last().unwrap();
            let artifact =
                tonk_identity::revocation::mint_root_revocation(account.clone(), chain, &target)
                    .await
                    .unwrap();
            let response = crate::router::http::post_cbor(&fixture.remote, &artifact)
                .await
                .unwrap();
            let receipt: tonk_account::customer::RevokeReceipt =
                serde_json::from_slice(&response.body).unwrap();
            assert_eq!(receipt.revoked, target);
        }

        let reader = SpaceWriter::open(&scratch_root(), &session, &space.did(), &fixture.remote)
            .await
            .unwrap();
        let (status, body) = query(
            &reader,
            &format!("writer.note:\n  this: ?n\n  tag: \"{}\"\n", fixture.run),
        )
        .await;
        assert!(
            !status.is_success(),
            "a revoked agent reads nothing: {body}"
        );
        assert!(
            body.to_string().contains("CREDENTIAL_REVOKED"),
            "refused for the revocation: {body}"
        );
        reader.close().await;
        fixture.stop().await;
    }
}
