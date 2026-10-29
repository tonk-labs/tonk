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
async fn opening_a_space_names_the_relay_and_the_remote() {
    let agent = Ed25519Signer::generate().await.unwrap();
    let space = Ed25519Signer::generate().await.unwrap();
    let (verified, writer) = verified(&agent, &space, &["get"]).await;
    let result = call(
        OPEN,
        &json!({ "space": space.did().to_string(), "path": "/notes" }),
        &verified,
        &writer,
        "",
        "session",
        ORIGIN,
        &TicketKey::derive("secret"),
    )
    .await
    .unwrap();
    assert_eq!(result["structuredContent"]["path"], "/notes");
    assert_eq!(result["structuredContent"]["edit"], false);
    assert_eq!(
        result["structuredContent"]["remote"],
        "https://tonk.example/ucan/"
    );
    assert_eq!(tools()[0]["_meta"]["ui"]["resourceUri"], RELAY_URI);
    assert_eq!(tools()[1]["_meta"]["ui"]["visibility"][0], "app");
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
    use http_body_util::BodyExt as _;
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

        /// A view's peer over the delegations its key was given.
        async fn view(&self, edit: bool) -> (HostedPeer, String) {
            let seed = Zeroizing::new(rand::random::<[u8; 32]>());
            let key = Ed25519Signer::import(&*seed).await.unwrap();
            let result = access(&self.verified, &self.agent.did(), &key.did(), edit).await;
            assert_eq!(result["isError"], false, "{result}");
            let peer = HostedPeer::open_in(
                Directory::At(scratch()),
                &seed,
                &chains(&result),
                HeldCaches::new(),
            )
            .await
            .unwrap();
            peer.mount(&self.agent.did(), &self.remote).await.unwrap();
            let base = format!("/api/repository/{}/branch/main", self.agent.did());
            (peer, base)
        }
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
