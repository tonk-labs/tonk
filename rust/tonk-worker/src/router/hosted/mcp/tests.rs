use std::collections::BTreeMap as Map;

use dialog_credentials::{Ed25519Signer, Signer};
use dialog_ucan_core::{Container, DelegationBuilder, Tag};
use dialog_varsig::Principal as _;
use ipld_core::ipld::Ipld;
use tonk_identity::agent::{
    AGENT_SESSION_TOKEN, AGENT_SESSION_TOKEN_VERSION, TOKEN_SEED, TOKEN_TYPE, TOKEN_VERSION,
    issue_agent_session_with_grants, mint_agent_session,
};
use zeroize::Zeroizing;

use super::*;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use wasm_bindgen_test::wasm_bindgen_test_configure;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
wasm_bindgen_test_configure!(run_in_service_worker);

const ORIGIN: &str = "https://tonk.example";

fn home() -> url::Url {
    url::Url::parse("https://tonk.example/ucan/").unwrap()
}

/// `space -> device -> agent` for each command: the shape the approval
/// page puts in a token, one chain per storage scope.
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

/// A token for a session of `agent`, granted `verbs` on `space`.
async fn token(agent: &Ed25519Signer, space: &Ed25519Signer, verbs: &[&str]) -> String {
    let grants = grants(space, &agent.did(), verbs).await;
    issue_agent_session_with_grants(agent.clone(), &home(), &grants)
        .await
        .unwrap()
        .token
}

/// Writers that must never be reached.
struct Unreachable;

impl Writers for Unreachable {
    async fn call(
        &self,
        _verified: &Verified,
        _bearer: &str,
        _mcp_session: &str,
        _space: &Did,
        _request: Request<Body>,
    ) -> Response<Body> {
        panic!("a refused call reached a writer")
    }
}

fn rpc_request(bearer: Option<&str>, session: Option<&str>, body: Value) -> Request<Body> {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(bearer) = bearer {
        request = request.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
    }
    if let Some(session) = session {
        request = request.header(MCP_SESSION_HEADER, session);
    }
    request.body(Body::from(body.to_string())).unwrap()
}

async fn answer(response: Response<Body>) -> (StatusCode, Value) {
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

fn initialize() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "0" },
        },
    })
}

async fn assert_unauthorized<W: Writers>(endpoint: &McpEndpoint<'_, W>, bearer: Option<&str>) {
    let response = endpoint
        .serve(rpc_request(bearer, None, initialize()))
        .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.headers()[header::WWW_AUTHENTICATE],
        "Bearer resource_metadata=\"https://tonk.example/.well-known/oauth-protected-resource/mcp\""
    );
}

#[dialog_common::test]
async fn a_missing_or_malformed_token_is_unauthorized() {
    let key = SessionKey::derive("test");
    let endpoint = McpEndpoint {
        origin: ORIGIN,
        key: &key,
        writers: &Unreachable,
    };
    assert_unauthorized(&endpoint, None).await;
    assert_unauthorized(&endpoint, Some("not-a-token")).await;
    assert_unauthorized(&endpoint, Some("Cxyz")).await;
}

#[dialog_common::test]
async fn a_token_whose_powerline_is_addressed_elsewhere_is_unauthorized() {
    let agent = Ed25519Signer::generate().await.unwrap();
    let seed = Zeroizing::new(rand::random::<[u8; 32]>());
    let session = Ed25519Signer::import(&*seed).await.unwrap();
    let other = Ed25519Signer::generate().await.unwrap();
    // The powerline names another key than the one the seed derives.
    let powerline = mint_agent_session(agent, &other.did(), &home())
        .await
        .unwrap();
    let meta = Map::from([
        (
            TOKEN_TYPE.to_owned(),
            Ipld::String(AGENT_SESSION_TOKEN.to_owned()),
        ),
        (
            TOKEN_VERSION.to_owned(),
            Ipld::Integer(AGENT_SESSION_TOKEN_VERSION.into()),
        ),
        (TOKEN_SEED.to_owned(), Ipld::Bytes(seed.to_vec())),
    ]);
    // The outer hop has to follow the powerline's audience to form a
    // chain at all, so `other` signs it, about itself.
    let outer = DelegationBuilder::new()
        .issuer(Signer::from(other.clone()))
        .audience(&other.did())
        .subject(UcanSubject::Specific(other.did()))
        .command(vec![])
        .meta(meta)
        .try_build()
        .await
        .unwrap();
    let forged = String::from_utf8(
        Container::from(&powerline.push(outer).unwrap())
            .encode(Tag::Base64Url)
            .unwrap(),
    )
    .unwrap();
    assert_ne!(session.did(), other.did());

    let key = SessionKey::derive("test");
    let endpoint = McpEndpoint {
        origin: ORIGIN,
        key: &key,
        writers: &Unreachable,
    };
    assert_unauthorized(&endpoint, Some(&forged)).await;
}

#[dialog_common::test]
async fn a_grant_not_issued_by_its_space_is_unauthorized() {
    let agent = Ed25519Signer::generate().await.unwrap();
    let space = Ed25519Signer::generate().await.unwrap();
    let impostor = Ed25519Signer::generate().await.unwrap();
    // Signed by the impostor, claiming the space as its subject.
    let forged = DelegationChain::new(
        DelegationBuilder::new()
            .issuer(Signer::from(impostor))
            .audience(&agent.did())
            .subject(UcanSubject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap(),
    );
    let token = issue_agent_session_with_grants(agent, &home(), &[forged])
        .await
        .unwrap()
        .token;

    let key = SessionKey::derive("test");
    let endpoint = McpEndpoint {
        origin: ORIGIN,
        key: &key,
        writers: &Unreachable,
    };
    assert_unauthorized(&endpoint, Some(&token)).await;
}

#[dialog_common::test]
async fn grants_set_each_spaces_access() {
    let agent = Ed25519Signer::generate().await.unwrap();
    let read = Ed25519Signer::generate().await.unwrap();
    let write = Ed25519Signer::generate().await.unwrap();
    let mut chains = grants(&read, &agent.did(), &["get"]).await;
    chains.extend(grants(&write, &agent.did(), &["get", "put"]).await);
    let token = issue_agent_session_with_grants(agent.clone(), &home(), &chains)
        .await
        .unwrap()
        .token;

    let verified = verify(&token).await.unwrap();
    let spaces = verified.spaces();
    assert_eq!(spaces.len(), 3);
    let own = &spaces[&agent.did()];
    assert!(own.agent && own.read && own.write);
    let only_read = &spaces[&read.did()];
    assert!(only_read.read && !only_read.write);
    assert_eq!(only_read.grants.len(), 3);
    let both = &spaces[&write.did()];
    assert!(both.read && both.write);
    assert_eq!(
        verified.writer_session(&write.did(), "mcp-1").grants.len(),
        6
    );
}

#[dialog_common::test]
fn session_ids_bind_to_their_token() {
    let key = SessionKey::derive("secret");
    let token = [1u8; 32];
    let other = [2u8; 32];
    let id = key.mint(&token);
    assert!(key.verify(&token, &id));
    assert!(!key.verify(&other, &id), "another token's id");
    assert!(
        !SessionKey::derive("another secret").verify(&token, &id),
        "another host's id"
    );
    assert!(!key.verify(&token, "made-up"));
    assert_ne!(key.mint(&token), id, "each initialize mints a new id");
}

#[dialog_common::test]
async fn a_session_id_of_another_token_is_refused() {
    let agent = Ed25519Signer::generate().await.unwrap();
    let space = Ed25519Signer::generate().await.unwrap();
    let first = token(&agent, &space, &["get"]).await;
    let second = token(&agent, &space, &["get"]).await;
    let key = SessionKey::derive("test");
    let endpoint = McpEndpoint {
        origin: ORIGIN,
        key: &key,
        writers: &Unreachable,
    };
    let response = endpoint
        .serve(rpc_request(Some(&first), None, initialize()))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let session = response.headers()[MCP_SESSION_HEADER]
        .to_str()
        .unwrap()
        .to_owned();

    let list = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
    let (status, body) = answer(
        endpoint
            .serve(rpc_request(Some(&first), Some(&session), list.clone()))
            .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<&str> = body["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "tonk_list_spaces",
            "tonk_query",
            "tonk_evaluate",
            "tonk_guide"
        ]
    );

    let (status, _) = answer(
        endpoint
            .serve(rpc_request(Some(&second), Some(&session), list.clone()))
            .await,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = answer(endpoint.serve(rpc_request(Some(&first), None, list)).await).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[dialog_common::test]
async fn evaluate_on_a_read_grant_is_refused_before_any_writer() {
    let agent = Ed25519Signer::generate().await.unwrap();
    let space = Ed25519Signer::generate().await.unwrap();
    let bearer = token(&agent, &space, &["get"]).await;
    let key = SessionKey::derive("test");
    let endpoint = McpEndpoint {
        origin: ORIGIN,
        key: &key,
        writers: &Unreachable,
    };
    let response = endpoint
        .serve(rpc_request(Some(&bearer), None, initialize()))
        .await;
    let session = response.headers()[MCP_SESSION_HEADER]
        .to_str()
        .unwrap()
        .to_owned();
    let call = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "tonk_evaluate",
            "arguments": { "space": space.did().to_string(), "document": "x!: {}" },
        },
    });
    let (status, body) = answer(
        endpoint
            .serve(rpc_request(Some(&bearer), Some(&session), call))
            .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["isError"], json!(true));
    let text = body["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("only read"), "{text}");
}

#[cfg(all(feature = "integration-tests", not(target_arch = "wasm32")))]
mod against_a_service {
    //! The endpoint over real space writers and a native access service:
    //! the MCP smoke test. In the Host Worker the same endpoint runs over
    //! Durable Objects, each holding one of these writers.

    use std::collections::HashMap;
    use std::sync::Arc;

    use dialog_common::helpers::Provisionable as _;
    use tonk_access_service::helpers::AccessServiceAddress;

    use super::super::super::SpaceWriter;
    use super::*;

    const SCHEMA: &str = "attribute!: &mcp-note-title
  description: Title
  the: mcp.note/title
  as: text
concept!: &mcp-note
  description: A note
  with:
    title: mcp-note-title
";

    /// One writer per space and writer DID, as the Durable Objects are.
    struct NativeWriters {
        remote: url::Url,
        open: tokio::sync::Mutex<HashMap<String, Arc<SpaceWriter>>>,
    }

    impl Writers for NativeWriters {
        async fn call(
            &self,
            verified: &Verified,
            _bearer: &str,
            mcp_session: &str,
            space: &Did,
            request: Request<Body>,
        ) -> Response<Body> {
            let session = verified.writer_session(space, mcp_session);
            let writer = {
                let mut open = self.open.lock().await;
                let name = format!(
                    "{space}/{}",
                    blake3::hash(&*super::super::super::writer_seed(
                        &session.seed,
                        mcp_session
                    ))
                );
                match open.get(&name) {
                    Some(writer) => writer.clone(),
                    None => {
                        let root = std::env::temp_dir()
                            .join(format!("mcp-test-{}", rand::random::<u64>()))
                            .to_string_lossy()
                            .into_owned();
                        let writer = Arc::new(
                            SpaceWriter::open(&root, &session, space, &self.remote)
                                .await
                                .unwrap(),
                        );
                        open.insert(name, writer.clone());
                        writer
                    }
                }
            };
            let (parts, body) = request.into_parts();
            let routed = |uri: &str| {
                Request::builder()
                    .method(Method::POST)
                    .uri(uri)
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(body)
                    .unwrap()
            };
            match parts.uri.path() {
                "/api/query" => writer.read(routed("/evaluate?transact=false")).await,
                "/api/evaluate" => writer.write(routed("/evaluate?transact=true")).await,
                other => panic!("no writer route {other}"),
            }
        }
    }

    struct Client<'a> {
        endpoint: &'a McpEndpoint<'a, NativeWriters>,
        bearer: String,
        session: String,
    }

    impl<'a> Client<'a> {
        async fn connect(endpoint: &'a McpEndpoint<'a, NativeWriters>, bearer: String) -> Self {
            let response = endpoint
                .serve(rpc_request(Some(&bearer), None, initialize()))
                .await;
            assert_eq!(response.status(), StatusCode::OK);
            let session = response.headers()[MCP_SESSION_HEADER]
                .to_str()
                .unwrap()
                .to_owned();
            let initialized = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
            let response = endpoint
                .serve(rpc_request(Some(&bearer), Some(&session), initialized))
                .await;
            assert_eq!(response.status(), StatusCode::ACCEPTED);
            Self {
                endpoint,
                bearer,
                session,
            }
        }

        /// Call a tool: its result and whether it is an error.
        async fn call(&self, name: &str, arguments: Value) -> (Value, bool) {
            let call = json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "tools/call",
                "params": { "name": name, "arguments": arguments },
            });
            let (status, body) = answer(
                self.endpoint
                    .serve(rpc_request(Some(&self.bearer), Some(&self.session), call))
                    .await,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let result = &body["result"];
            let text = result["content"][0]["text"].as_str().unwrap_or_default();
            let value = serde_json::from_str(text).unwrap_or(json!(text));
            (value, result["isError"] == json!(true))
        }
    }

    #[dialog_common::test]
    async fn a_read_grant_queries_and_a_write_grant_evaluates() {
        let service = AccessServiceAddress::start(Default::default())
            .await
            .unwrap();
        let agent = Ed25519Signer::generate().await.unwrap();
        let space = Ed25519Signer::generate().await.unwrap();
        for subject in [agent.did(), space.did()] {
            service
                .address
                .provision_subject(subject.as_ref())
                .await
                .unwrap();
        }
        let remote = url::Url::parse(&service.address.ucan_endpoint()).unwrap();
        let writers = NativeWriters {
            remote,
            open: Default::default(),
        };
        let key = SessionKey::derive("test");
        let endpoint = McpEndpoint {
            origin: ORIGIN,
            key: &key,
            writers: &writers,
        };
        let subject = space.did().to_string();
        let run: u64 = rand::random();
        let note = format!("mcp-note!:\n  this: urn:mcp:{run}\n  title: \"note {run}\"\n");
        let find = format!("mcp.note:\n  this: urn:mcp:{run}\n  title: ?title\n");

        // Read and write: the schema and a note land.
        let writer = Client::connect(&endpoint, token(&agent, &space, &["get", "put"]).await).await;
        let (spaces, error) = writer.call("tonk_list_spaces", json!({})).await;
        assert!(!error, "{spaces}");
        let listed = spaces["spaces"].as_array().unwrap();
        assert!(
            listed.iter().any(|entry| entry["subject"] == json!(subject)
                && entry["access"] == json!("read-write"))
        );
        for document in [SCHEMA.to_owned(), note.clone()] {
            let (result, error) = writer
                .call(
                    "tonk_evaluate",
                    json!({ "space": subject, "document": document }),
                )
                .await;
            assert!(!error, "evaluate: {result}");
        }

        // Read only, through another OAuth session: it sees the note, and
        // its evaluate is refused and writes nothing.
        let reader = Client::connect(&endpoint, token(&agent, &space, &["get"]).await).await;
        let (found, error) = reader
            .call("tonk_query", json!({ "space": subject, "document": find }))
            .await;
        assert!(!error, "query: {found}");
        assert!(
            found.to_string().contains(&format!("note {run}")),
            "{found}"
        );

        let other = format!("mcp-note!:\n  this: urn:mcp:{run}:refused\n  title: \"refused\"\n");
        let (refused, error) = reader
            .call(
                "tonk_evaluate",
                json!({ "space": subject, "document": other }),
            )
            .await;
        assert!(error, "a read grant evaluated: {refused}");
        let (after, _) = writer
            .call(
                "tonk_query",
                json!({
                    "space": subject,
                    "document": format!("mcp.note:\n  this: urn:mcp:{run}:refused\n  title: ?title\n"),
                }),
            )
            .await;
        assert!(!after.to_string().contains("\"refused\""), "{after}");

        // A space the token holds no grant for.
        let elsewhere = Ed25519Signer::generate().await.unwrap().did().to_string();
        let (_, error) = writer
            .call(
                "tonk_query",
                json!({ "space": elsewhere, "document": find }),
            )
            .await;
        assert!(error);

        let open: Vec<Arc<SpaceWriter>> =
            writers.open.lock().await.drain().map(|(_, w)| w).collect();
        drop(open);
        service.stop().await.unwrap();
    }
}
