use dialog_credentials::Ed25519Signer;
use dialog_ucan_core::Container;
use dialog_varsig::Principal as _;
use tonk_identity::agent::issue_agent_session_with_grants;

use super::*;

const ORIGIN: &str = "https://tonk.example";

fn home() -> url::Url {
    url::Url::parse("https://tonk.example/ucan/").unwrap()
}

/// `space -> device -> agent` for each `verb` and storage scope: the shape
/// the approval page retains in the agent space.
async fn grants(space: &Ed25519Signer, agent: &Did, verbs: &[&str]) -> Vec<DelegationChain> {
    let device = Ed25519Signer::generate().await.unwrap();
    let mut chains = Vec::new();
    for verb in verbs {
        for (kind, item) in [
            ("memory", "cell"),
            ("archive", "block"),
            ("archive", "blob"),
        ] {
            let command: Vec<String> = vec!["use".into(), (*verb).into(), kind.into(), item.into()];
            let root = DelegationBuilder::new()
                .issuer(Signer::from(space.clone()))
                .audience(&device.did())
                .subject(UcanSubject::Specific(space.did()))
                .command(command.clone())
                .try_build()
                .await
                .unwrap();
            let leaf = DelegationBuilder::new()
                .issuer(Signer::from(device.clone()))
                .audience(agent)
                .subject(UcanSubject::Specific(space.did()))
                .command(command)
                .try_build()
                .await
                .unwrap();
            chains.push(DelegationChain::try_from(vec![root, leaf]).unwrap());
        }
    }
    chains
}

/// The agent space's writer, as the front reaches it: answers what the
/// agent space holds, `grants` for `space`.
struct AgentWriter {
    space: Did,
    grants: Vec<DelegationChain>,
}

impl Writers for AgentWriter {
    async fn call(
        &self,
        verified: &Verified,
        _bearer: &str,
        _mcp_session: &str,
        space: &Did,
        request: Request<Body>,
    ) -> Response<Body> {
        assert_eq!(space, verified.agent(), "only the agent writer is asked");
        let reach = |verb| {
            self.grants
                .iter()
                .any(|chain| reaches(&command_of(chain), verb))
        };
        let body = match request.uri().path() {
            ACCESS_ROUTE => {
                let mut access = AgentAccess::default();
                if !self.grants.is_empty() {
                    access.spaces.insert(
                        self.space.to_string(),
                        SpaceAccess {
                            read: reach("get"),
                            write: reach("put"),
                            ..SpaceAccess::default()
                        },
                    );
                }
                serde_json::to_value(access).unwrap()
            }
            CHAINS_ROUTE => {
                let asked = request.uri().query().unwrap_or_default();
                assert_eq!(asked, format!("space={}", self.space));
                chains_answer(&self.grants).unwrap()
            }
            other => panic!("no agent writer route {other}"),
        };
        Response::new(Body::from(body.to_string()))
    }
}

/// A verified token for a session of `agent`, and its agent space's
/// writer holding grants of `verbs` on `space`.
async fn verified(
    agent: &Ed25519Signer,
    space: &Ed25519Signer,
    verbs: &[&str],
) -> (Verified, AgentWriter) {
    let grants = grants(space, &agent.did(), verbs).await;
    let token = issue_agent_session_with_grants(agent.clone(), &home(), &[])
        .await
        .unwrap()
        .token;
    let verified = super::super::mcp::verify(&token).await.unwrap();
    let writer = AgentWriter {
        space: space.did(),
        grants,
    };
    (verified, writer)
}

async fn access(
    (verified, writer): &(Verified, AgentWriter),
    space: &Did,
    audience: &Did,
    edit: bool,
) -> Value {
    call(
        ACCESS,
        &json!({ "space": space.to_string(), "audience": audience.to_string(), "edit": edit }),
        verified,
        writer,
        "",
        "session",
        ORIGIN,
        &TicketKey::derive("secret"),
    )
    .await
    .unwrap()
}

fn chains(result: &Value) -> Vec<DelegationChain> {
    result["structuredContent"]["delegations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|encoded| {
            let container = Container::decode(encoded.as_str().unwrap().as_bytes()).unwrap();
            DelegationChain::try_from(container).unwrap()
        })
        .collect()
}

#[dialog_common::test]
async fn a_read_only_view_gets_only_read_commands_of_its_space() {
    let agent = Ed25519Signer::generate().await.unwrap();
    let space = Ed25519Signer::generate().await.unwrap();
    let view = Ed25519Signer::generate().await.unwrap();
    let verified = verified(&agent, &space, &["get", "put"]).await;
    let result = access(&verified, &space.did(), &view.did(), false).await;
    assert_eq!(result["isError"], false, "{result}");
    let chains = chains(&result);
    assert_eq!(chains.len(), 3, "the three read grants, not the puts");
    for chain in &chains {
        assert_eq!(chain.audience(), &view.did());
        assert_eq!(chain.issuer(), &space.did(), "rooted at the space");
        assert_eq!(chain.subject(), Some(&space.did()));
        assert!(
            chain.ability().starts_with("/use/get/"),
            "{}",
            chain.ability()
        );
        assert!(chain.expiration().is_some());
    }
}

#[dialog_common::test]
async fn an_editing_view_gets_the_puts_too() {
    let agent = Ed25519Signer::generate().await.unwrap();
    let space = Ed25519Signer::generate().await.unwrap();
    let view = Ed25519Signer::generate().await.unwrap();
    let verified = verified(&agent, &space, &["get", "put"]).await;
    let result = access(&verified, &space.did(), &view.did(), true).await;
    let abilities: Vec<String> = chains(&result).iter().map(|c| c.ability()).collect();
    assert_eq!(abilities.len(), 6, "{abilities:?}");
    assert!(abilities.iter().any(|a| a.starts_with("/use/put/")));
}

#[dialog_common::test]
async fn a_view_of_a_read_only_space_cannot_edit() {
    let agent = Ed25519Signer::generate().await.unwrap();
    let space = Ed25519Signer::generate().await.unwrap();
    let view = Ed25519Signer::generate().await.unwrap();
    let verified = verified(&agent, &space, &["get"]).await;
    let result = access(&verified, &space.did(), &view.did(), true).await;
    assert_eq!(result["isError"], true, "{result}");
}

#[dialog_common::test]
async fn the_agent_space_is_delegated_through_the_powerline() {
    let agent = Ed25519Signer::generate().await.unwrap();
    let space = Ed25519Signer::generate().await.unwrap();
    let view = Ed25519Signer::generate().await.unwrap();
    let verified = verified(&agent, &space, &["get"]).await;
    let result = access(&verified, &agent.did(), &view.did(), false).await;
    let chains = chains(&result);
    assert_eq!(chains.len(), 1);
    assert_eq!(chains[0].issuer(), &agent.did());
    assert_eq!(
        chains[0].proofs().last().unwrap().subject(),
        &UcanSubject::Specific(agent.did()),
        "the last hop names the agent space"
    );
    assert_eq!(chains[0].ability(), "/use/get");
}

#[dialog_common::test]
async fn a_space_the_agent_space_does_not_reach_is_refused() {
    let agent = Ed25519Signer::generate().await.unwrap();
    let space = Ed25519Signer::generate().await.unwrap();
    let other = Ed25519Signer::generate().await.unwrap();
    let view = Ed25519Signer::generate().await.unwrap();
    let verified = verified(&agent, &space, &["get"]).await;
    let result = access(&verified, &other.did(), &view.did(), false).await;
    assert_eq!(result["isError"], true, "{result}");
}

#[dialog_common::test]
async fn a_display_names_its_entities_the_relay_and_the_remote() {
    let agent = Ed25519Signer::generate().await.unwrap();
    let space = Ed25519Signer::generate().await.unwrap();
    let (verified, writer) = verified(&agent, &space, &["get"]).await;
    let display = |items: Value| {
        let arguments = json!({ "space": space.did().to_string(), "items": items });
        let (verified, writer) = (&verified, &writer);
        async move {
            call(
                DISPLAY,
                &arguments,
                verified,
                writer,
                "",
                "session",
                ORIGIN,
                &TicketKey::derive("secret"),
            )
            .await
            .unwrap()
        }
    };
    let result = display(json!([
        { "entity": "urn:e2e:note", "model": "e2e-note", "view": "e2e-note-card" }
    ]))
    .await;
    assert_eq!(
        result["structuredContent"]["items"],
        json!([{ "entity": "urn:e2e:note", "model": "e2e-note", "view": "e2e-note-card" }])
    );
    assert_eq!(result["structuredContent"]["edit"], false);
    assert_eq!(
        result["structuredContent"]["remote"],
        "https://tonk.example/ucan/"
    );
    assert_eq!(tools()[0]["_meta"]["ui"]["resourceUri"], RELAY_URI);
    assert_eq!(tools()[1]["_meta"]["ui"]["visibility"][0], "app");

    for items in [
        json!([]),
        json!([{ "entity": "urn:e2e:note" }]),
        json!([{ "entity": "", "model": "e2e-note" }]),
        json!([{ "entity": 1, "model": "e2e-note" }]),
        json!((0..21).map(|n| json!({ "entity": format!("urn:{n}"), "model": "m" })).collect::<Vec<_>>()),
    ] {
        let result = display(items.clone()).await;
        assert_eq!(result["isError"], true, "{items} was displayed: {result}");
    }
}

#[test]
fn the_relay_is_filled_with_the_origin_it_was_reached_on() {
    let read = read_resource(RELAY_URI, "https://tonk.example/").unwrap();
    let text = read["contents"][0]["text"].as_str().unwrap();
    assert!(text.contains("const origin = \"https://tonk.example\""));
    assert!(!text.contains("__TONK_MCP_ORIGIN__"));
    assert!(text.contains("framed = false"), "the host's copy relays");
    assert_eq!(
        read["contents"][0]["_meta"]["ui"]["csp"]["frameDomains"][0], "https://tonk.example",
        "route A asks to frame the provider"
    );
    assert_eq!(
        read["contents"][0]["_meta"]["ui"]["csp"]["connectDomains"],
        json!(["https://tonk.example", "wss://tonk.example"]),
        "the view reaches the provider and its live socket"
    );
    assert_eq!(resources()[0]["uri"], RELAY_URI);
    assert!(read_resource("ui://tonk/other.html", ORIGIN).is_none());
}

#[cfg(all(feature = "integration-tests", not(target_arch = "wasm32")))]
mod against_a_service {
    //! A view's peer, acting as the view's own key with the delegations
    //! `tonk_view_access` signs, against the native access service.

    use axum::body::Body;
    use axum::http::{Method, Request, Response, StatusCode, header};
    use dialog_common::helpers::{Provisionable as _, Service};
    use dialog_effects::storage::Directory;
    use dialog_repository::HeldCaches;
    use tonk_access_service::helpers::{AccessServer, AccessServiceAddress};
    use zeroize::Zeroizing;

    use super::super::super::{HostedPeer, SpaceWriter, WriterSession};
    use super::*;

    const NOTE: &str = "attribute!: &view-note-title
  description: Title
  the: view.note/title
  as: text
concept!: &view-note
  description: A note
  with:
    title: view-note-title
";

    fn request(method: Method, uri: &str, body: impl Into<Body>) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "text/plain")
            .body(body.into())
            .unwrap()
    }

    async fn json(response: Response<Body>) -> (StatusCode, Value) {
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let value = serde_json::from_slice(&body)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned()));
        (status, value)
    }

    fn scratch() -> String {
        std::env::temp_dir()
            .join(format!("view-test-{}", rand::random::<u64>()))
            .to_string_lossy()
            .into_owned()
    }

    /// An agent space served by a fresh access service, holding one note
    /// its session wrote, and the token's verified session.
    struct Fixture {
        service: Service<AccessServiceAddress, AccessServer>,
        agent: Ed25519Signer,
        remote: url::Url,
        verified: (Verified, AgentWriter),
    }

    impl Fixture {
        async fn start() -> Self {
            let service = AccessServiceAddress::start(Default::default())
                .await
                .unwrap();
            let agent = Ed25519Signer::generate().await.unwrap();
            service
                .address
                .provision_subject(agent.did().as_ref())
                .await
                .unwrap();
            let remote = url::Url::parse(&service.address.ucan_endpoint()).unwrap();
            let token = issue_agent_session_with_grants(agent.clone(), &remote, &[])
                .await
                .unwrap()
                .token;
            let verified = super::super::super::mcp::verify(&token).await.unwrap();
            let session: WriterSession = verified.writer_session("agent");
            let writer = SpaceWriter::open(&scratch(), &session, &agent.did(), &remote)
                .await
                .unwrap();
            let document =
                format!("{NOTE}view-note!:\n  this: urn:view:note\n  title: \"from the agent\"\n");
            let (status, body) = json(
                writer
                    .write(request(Method::POST, "/evaluate?transact=true", document))
                    .await,
            )
            .await;
            assert!(status.is_success(), "seed: {status}: {body}");
            writer.close().await;
            let writer = AgentWriter {
                space: agent.did(),
                grants: Vec::new(),
            };
            Self {
                service,
                agent,
                remote,
                verified: (verified, writer),
            }
        }

        /// A view's peer of the agent space over the delegations its key
        /// was given.
        async fn view(&self, edit: bool) -> (HostedPeer, String) {
            let (verified, writer) = &self.verified;
            self.view_of(verified, writer, &self.agent.did(), edit)
                .await
        }

        /// A view's peer of `space`, its delegations signed by the
        /// session `verified` from what `writers` answers.
        async fn view_of(
            &self,
            verified: &Verified,
            writers: &impl Writers,
            space: &Did,
            edit: bool,
        ) -> (HostedPeer, String) {
            let seed = Zeroizing::new(rand::random::<[u8; 32]>());
            let key = Ed25519Signer::import(&*seed).await.unwrap();
            let result = call(
                ACCESS,
                &json!({ "space": space.to_string(), "audience": key.did().to_string(), "edit": edit }),
                verified,
                writers,
                "",
                "session",
                ORIGIN,
                &TicketKey::derive("secret"),
            )
            .await
            .unwrap();
            assert_eq!(result["isError"], false, "{result}");
            let peer = HostedPeer::open_in(
                Directory::At(scratch()),
                &seed,
                &chains(&result),
                HeldCaches::new(),
            )
            .await
            .unwrap();
            peer.mount(space, &self.remote).await.unwrap();
            let base = format!("/api/repository/{space}/branch/main");
            (peer, base)
        }
    }

    /// The agent space's own writer, answering the front's access routes
    /// as the Host Worker's does.
    struct Served(SpaceWriter);

    impl Writers for Served {
        async fn call(
            &self,
            _verified: &Verified,
            _bearer: &str,
            _mcp_session: &str,
            _space: &Did,
            request: Request<Body>,
        ) -> Response<Body> {
            let answer = match request.uri().path() {
                ACCESS_ROUTE => serde_json::to_value(self.0.agent_access().await.unwrap()).unwrap(),
                CHAINS_ROUTE => {
                    let query = request.uri().query().unwrap_or_default();
                    let space = query.strip_prefix("space=").unwrap().parse().unwrap();
                    chains_answer(&self.0.space_chains(&space).await.unwrap()).unwrap()
                }
                other => panic!("no agent writer route {other}"),
            };
            Response::new(Body::from(answer.to_string()))
        }
    }

    /// Grants of `verb` on `space` to `agent`, as the approval retains
    /// them: `space -> device -> agent`, one per storage scope.
    async fn grant(space: &Ed25519Signer, agent: &Did, verb: &str) -> Vec<DelegationChain> {
        super::grants(space, agent, &[verb]).await
    }

    #[dialog_common::test]
    async fn a_view_reads_a_granted_space_through_the_grants_the_agent_space_holds() {
        let fixture = Fixture::start().await;
        let agent = fixture.agent.did();
        let garden = Ed25519Signer::generate().await.unwrap();
        fixture
            .service
            .address
            .provision_subject(garden.did().as_ref())
            .await
            .unwrap();
        // The garden's owner writes a note there.
        let owner = issue_agent_session_with_grants(garden.clone(), &fixture.remote, &[])
            .await
            .unwrap()
            .token;
        let owner = super::super::super::mcp::verify(&owner).await.unwrap();
        let owner = SpaceWriter::open(
            &scratch(),
            &owner.writer_session("owner"),
            &garden.did(),
            &fixture.remote,
        )
        .await
        .unwrap();
        let document =
            format!("{NOTE}view-note!:\n  this: urn:view:garden\n  title: \"in the garden\"\n");
        let (status, body) = json(
            owner
                .write(request(Method::POST, "/evaluate?transact=true", document))
                .await,
        )
        .await;
        assert!(status.is_success(), "seed: {status}: {body}");
        owner.close().await;
        // The approval retains a read grant of it in the agent space.
        let (verified, _) = &fixture.verified;
        let session = verified.writer_session("agent");
        let own = SpaceWriter::open(&scratch(), &session, &agent, &fixture.remote)
            .await
            .unwrap();
        own.approve(&grant(&garden, &agent, "get").await, &[])
            .await
            .unwrap();
        let served = Served(own);

        let (view, base) = fixture
            .view_of(verified, &served, &garden.did(), false)
            .await;
        let (status, body) = json(
            view.call(request(Method::POST, &format!("{base}/sync/pull"), ""))
                .await,
        )
        .await;
        assert!(status.is_success(), "pull: {status}: {body}");
        let (status, body) = json(
            view.call(request(
                Method::POST,
                &format!("{base}/evaluate?transact=false"),
                "view.note:\n  this: ?n\n  title: ?t\n",
            ))
            .await,
        )
        .await;
        assert!(status.is_success(), "query: {status}: {body}");
        assert!(body.to_string().contains("in the garden"), "{body}");

        // Editing a space granted only to read is refused.
        let key = Ed25519Signer::generate().await.unwrap();
        let refused = call(
            ACCESS,
            &json!({ "space": garden.did().to_string(), "audience": key.did().to_string(), "edit": true }),
            verified,
            &served,
            "",
            "session",
            ORIGIN,
            &TicketKey::derive("secret"),
        )
        .await
        .unwrap();
        assert_eq!(refused["isError"], true, "{refused}");
        view.close().await;
        served.0.close().await;
        fixture.service.stop().await.unwrap();
    }

    /// Route A: a framed view's peer is the provider's ordinary worker,
    /// holding nothing; the frame hands it the delegations to its own
    /// profile through `/api/view/access`, and it reads the space.
    #[dialog_common::test]
    async fn a_framed_peer_takes_its_delegation_and_reads_the_space() {
        let fixture = Fixture::start().await;
        let seed = Zeroizing::new(rand::random::<[u8; 32]>());
        let peer = HostedPeer::open_in(Directory::At(scratch()), &seed, &[], HeldCaches::new())
            .await
            .unwrap();
        let (verified, writer) = &fixture.verified;
        let space = fixture.agent.did();
        let result = call(
            ACCESS,
            &json!({ "space": space.to_string(), "audience": peer.did().await.to_string() }),
            verified,
            writer,
            "",
            "session",
            ORIGIN,
            &TicketKey::derive("secret"),
        )
        .await
        .unwrap();
        let accept = |delegations: &Value| {
            Request::builder()
                .method(Method::POST)
                .uri("/api/view/access")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "space": space.to_string(),
                        "remote": fixture.remote.to_string(),
                        "delegations": delegations,
                    })
                    .to_string(),
                ))
                .unwrap()
        };
        // Chains to another key are refused.
        let stranger = Ed25519Signer::generate().await.unwrap();
        let other = access(&fixture.verified, &space, &stranger.did(), false).await;
        let (status, body) = json(
            peer.call(accept(&other["structuredContent"]["delegations"]))
                .await,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

        let (status, body) = json(
            peer.call(accept(&result["structuredContent"]["delegations"]))
                .await,
        )
        .await;
        assert!(status.is_success(), "accept: {status}: {body}");
        let base = format!("/api/repository/{space}/branch/main");
        let (status, body) = json(
            peer.call(request(Method::POST, &format!("{base}/sync/pull"), ""))
                .await,
        )
        .await;
        assert!(status.is_success(), "pull: {status}: {body}");
        let (status, body) = json(
            peer.call(request(
                Method::POST,
                &format!("{base}/evaluate?transact=false"),
                "view.note:\n  this: ?n\n  title: ?t\n",
            ))
            .await,
        )
        .await;
        assert!(status.is_success(), "query: {status}: {body}");
        assert!(body.to_string().contains("from the agent"), "{body}");
        // A renewed delegation is accepted over the same replica.
        let (status, body) = json(
            peer.call(accept(&result["structuredContent"]["delegations"]))
                .await,
        )
        .await;
        assert!(status.is_success(), "renew: {status}: {body}");
        peer.close().await;
        fixture.service.stop().await.unwrap();
    }

    #[dialog_common::test]
    async fn a_view_reads_its_space_as_its_own_key() {
        let fixture = Fixture::start().await;
        let (view, base) = fixture.view(false).await;
        let (status, body) = json(
            view.call(request(Method::POST, &format!("{base}/sync/pull"), ""))
                .await,
        )
        .await;
        assert!(status.is_success(), "pull: {status}: {body}");
        let (status, body) = json(
            view.call(request(
                Method::POST,
                &format!("{base}/evaluate?transact=false"),
                "view.note:\n  this: ?n\n  title: ?t\n",
            ))
            .await,
        )
        .await;
        assert!(status.is_success(), "query: {status}: {body}");
        assert!(body.to_string().contains("from the agent"), "{body}");
        view.close().await;
        fixture.service.stop().await.unwrap();
    }

    #[dialog_common::test]
    async fn a_read_only_view_cannot_publish() {
        let fixture = Fixture::start().await;
        let (view, base) = fixture.view(false).await;
        let (status, _) = json(
            view.call(request(Method::POST, &format!("{base}/sync/pull"), ""))
                .await,
        )
        .await;
        assert!(status.is_success());
        let (status, body) = json(
            view.call(request(
                Method::POST,
                &format!("{base}/evaluate?transact=true"),
                "view-note!:\n  this: urn:view:other\n  title: \"from the view\"\n",
            ))
            .await,
        )
        .await;
        assert!(status.is_success(), "local commit: {status}: {body}");
        let (status, body) = json(
            view.call(request(Method::POST, &format!("{base}/sync/push"), ""))
                .await,
        )
        .await;
        assert!(
            !status.is_success(),
            "a read-only view's push lands: {body}"
        );
        // The worker reports the remote's refusal only as a failed sync;
        // the same push from a view opened to edit lands (below), so the
        // read-only delegation is what the remote refused.
        assert_eq!(body["error"]["code"], "UPSTREAM_ERROR", "{body}");
        view.close().await;
        fixture.service.stop().await.unwrap();
    }

    #[dialog_common::test]
    async fn an_editing_view_publishes_as_its_own_key() {
        let fixture = Fixture::start().await;
        let (view, base) = fixture.view(true).await;
        let (status, _) = json(
            view.call(request(Method::POST, &format!("{base}/sync/pull"), ""))
                .await,
        )
        .await;
        assert!(status.is_success());
        let (status, body) = json(
            view.call(request(
                Method::POST,
                &format!("{base}/evaluate?transact=true"),
                "view-note!:\n  this: urn:view:edit\n  title: \"from the view\"\n",
            ))
            .await,
        )
        .await;
        assert!(status.is_success(), "commit: {status}: {body}");
        assert_eq!(
            body["revision_after"]["issuer"]
                .as_str()
                .map(|s| s.is_empty()),
            Some(false),
            "{body}"
        );
        let (status, body) = json(
            view.call(request(Method::POST, &format!("{base}/sync/push"), ""))
                .await,
        )
        .await;
        assert!(status.is_success(), "push: {status}: {body}");
        view.close().await;
        fixture.service.stop().await.unwrap();
    }
}
