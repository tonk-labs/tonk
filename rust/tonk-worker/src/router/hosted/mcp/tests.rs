use std::collections::BTreeMap as Map;

use dialog_credentials::{Ed25519Signer, Signer};
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_ucan_core::time::Timestamp;
use dialog_ucan_core::{Container, DelegationBuilder, DelegationChain, Tag};
use dialog_varsig::Principal as _;
use ipld_core::ipld::Ipld;
use tonk_identity::agent::{
    AGENT_SESSION_TOKEN, AGENT_SESSION_TOKEN_VERSION, TOKEN_SEED, TOKEN_TYPE, TOKEN_VERSION,
    issue_agent_session, issue_agent_session_with_grants, mint_agent_session,
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
/// retains in the agent space, one chain per storage scope. The leaf
/// expires at `expires` (Unix seconds) when given.
async fn grants(
    space: &Ed25519Signer,
    agent: &Did,
    verbs: &[&str],
    expires: Option<u64>,
) -> Vec<DelegationChain> {
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
                .command(command);
            let leaf = match expires {
                Some(at) => leaf
                    .expiration(Timestamp::try_from(at as i128).unwrap())
                    .try_build()
                    .await
                    .unwrap(),
                None => leaf.try_build().await.unwrap(),
            };
            chains.push(DelegationChain::try_from(vec![root, leaf]).unwrap());
        }
    }
    chains
}

/// A token for a new session of `agent`: the powerline and the seed.
async fn token(agent: &Ed25519Signer, home: &url::Url) -> String {
    issue_agent_session(agent.clone(), home)
        .await
        .unwrap()
        .token
}

/// Writers that answer the agent space's grants with `access` and must
/// never be reached for anything else.
struct Granting {
    agent: Did,
    access: AgentAccess,
}

impl Writers for Granting {
    async fn call(
        &self,
        _verified: &Verified,
        _bearer: &str,
        _mcp_session: &str,
        space: &Did,
        request: Request<Body>,
    ) -> Response<Body> {
        assert_eq!(space, &self.agent, "a refused call reached a writer");
        assert_eq!(
            request.uri().path(),
            ACCESS_ROUTE,
            "a refused call reached a writer"
        );
        rpc(StatusCode::OK, serde_json::to_value(&self.access).unwrap())
    }
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

fn endpoint<'a, W: Writers>(
    key: &'a SessionKey,
    tickets: &'a crate::router::hosted::view::TicketKey,
    writers: &'a W,
) -> McpEndpoint<'a, W> {
    McpEndpoint {
        origin: ORIGIN,
        key,
        tickets,
        writers,
    }
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

/// Open an MCP session on `endpoint` with `bearer`: its id.
async fn connect<W: Writers>(endpoint: &McpEndpoint<'_, W>, bearer: &str) -> String {
    let response = endpoint
        .serve(rpc_request(Some(bearer), None, initialize()))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    response.headers()[MCP_SESSION_HEADER]
        .to_str()
        .unwrap()
        .to_owned()
}

/// Call a tool: its text and whether it is an error.
async fn call_tool<W: Writers>(
    endpoint: &McpEndpoint<'_, W>,
    bearer: &str,
    session: &str,
    name: &str,
    arguments: Value,
) -> (String, bool) {
    let call = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments },
    });
    let (status, body) = answer(
        endpoint
            .serve(rpc_request(Some(bearer), Some(session), call))
            .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let text = body["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    (text, body["result"]["isError"] == json!(true))
}

/// The agent space alone, as its writer answers it.
fn own(agent: &Did) -> AgentAccess {
    let mut access = AgentAccess::default();
    access.spaces.insert(
        agent.to_string(),
        SpaceAccess {
            agent: true,
            read: true,
            write: true,
            name: Some("Claude".into()),
            ..SpaceAccess::default()
        },
    );
    access
}

#[dialog_common::test]
async fn a_missing_or_malformed_token_is_unauthorized() {
    let key = SessionKey::derive("test");
    let tickets = crate::router::hosted::view::TicketKey::derive("test");
    let endpoint = endpoint(&key, &tickets, &Unreachable);
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
    let tickets = crate::router::hosted::view::TicketKey::derive("test");
    let endpoint = endpoint(&key, &tickets, &Unreachable);
    assert_unauthorized(&endpoint, Some(&forged)).await;
}

/// A grant a token still carries (the shape of an older approval) opens
/// nothing: access is what the agent space holds.
#[dialog_common::test]
async fn a_grant_in_the_token_opens_no_space() {
    let agent = Ed25519Signer::generate().await.unwrap();
    let space = Ed25519Signer::generate().await.unwrap();
    let carried = grants(&space, &agent.did(), &["get", "put"], None).await;
    let bearer = issue_agent_session_with_grants(agent.clone(), &home(), &carried)
        .await
        .unwrap()
        .token;
    let writers = Granting {
        agent: agent.did(),
        access: own(&agent.did()),
    };
    let key = SessionKey::derive("test");
    let tickets = crate::router::hosted::view::TicketKey::derive("test");
    let endpoint = endpoint(&key, &tickets, &writers);
    let session = connect(&endpoint, &bearer).await;

    for tool in ["tonk_query", "tonk_evaluate"] {
        let (text, error) = call_tool(
            &endpoint,
            &bearer,
            &session,
            tool,
            json!({ "space": space.did().to_string(), "document": "x: {}" }),
        )
        .await;
        assert!(error, "{tool} on an ungranted space: {text}");
        assert!(text.contains("has no access"), "{text}");
    }
    let (text, error) =
        call_tool(&endpoint, &bearer, &session, "tonk_list_spaces", json!({})).await;
    assert!(!error, "{text}");
    let listed: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(listed["spaces"].as_array().unwrap().len(), 1, "{listed}");
}

#[dialog_common::test]
async fn the_list_puts_home_first_with_names_and_expiry() {
    let agent = Ed25519Signer::generate().await.unwrap();
    let notes = Ed25519Signer::generate().await.unwrap().did();
    let plans = Ed25519Signer::generate().await.unwrap().did();
    let gone = Ed25519Signer::generate().await.unwrap().did();
    let mut access = own(&agent.did());
    access.spaces.insert(
        notes.to_string(),
        SpaceAccess {
            read: true,
            name: Some("Notes".into()),
            expires: Some(1_801_079_968),
            ..SpaceAccess::default()
        },
    );
    access.spaces.insert(
        plans.to_string(),
        SpaceAccess {
            read: true,
            write: true,
            ..SpaceAccess::default()
        },
    );
    access.spaces.insert(
        gone.to_string(),
        SpaceAccess {
            lapsed: 1,
            ..SpaceAccess::default()
        },
    );
    let writers = Granting {
        agent: agent.did(),
        access,
    };
    let key = SessionKey::derive("test");
    let tickets = crate::router::hosted::view::TicketKey::derive("test");
    let endpoint = endpoint(&key, &tickets, &writers);
    let bearer = token(&agent, &home()).await;
    let session = connect(&endpoint, &bearer).await;

    let (text, error) =
        call_tool(&endpoint, &bearer, &session, "tonk_list_spaces", json!({})).await;
    assert!(!error, "{text}");
    let listed: Value = serde_json::from_str(&text).unwrap();
    let spaces = listed["spaces"].as_array().unwrap();
    assert_eq!(spaces.len(), 3, "a lapsed space is not listed: {listed}");
    assert_eq!(spaces[0]["subject"], json!(agent.did().to_string()));
    assert_eq!(spaces[0]["home"], json!(true));
    assert_eq!(spaces[0]["access"], json!("read-write"));
    assert_eq!(spaces[0]["name"], json!("Claude"));
    assert!(spaces[0]["about"].is_string());
    let entry = |did: &Did| {
        spaces
            .iter()
            .find(|entry| entry["subject"] == json!(did.to_string()))
            .unwrap()
            .clone()
    };
    assert_eq!(
        entry(&notes),
        json!({
            "subject": notes.to_string(),
            "access": "read",
            "name": "Notes",
            "expires": "2027-01-27T19:59:28Z",
        })
    );
    assert_eq!(
        entry(&plans),
        json!({ "subject": plans.to_string(), "access": "read-write" })
    );

    // The lapsed space is named by the error a call on it gets.
    let (text, error) = call_tool(
        &endpoint,
        &bearer,
        &session,
        "tonk_query",
        json!({ "space": gone.to_string(), "document": "x: {}" }),
    )
    .await;
    assert!(error);
    assert!(text.contains("has lapsed"), "{text}");
}

#[dialog_common::test]
fn a_space_mounts_only_at_the_configured_origin() {
    let configured = "https://tonk.example/ucan/";
    assert_eq!(
        mount_address(Some("https://tonk.example/ucan/"), configured),
        configured
    );
    assert_eq!(
        mount_address(Some("https://tonk.example/other/ucan/"), configured),
        "https://tonk.example/other/ucan/",
        "another path on the configured origin"
    );
    for foreign in [
        "https://elsewhere.example/ucan/",
        "http://tonk.example/ucan/",
        "https://tonk.example:8443/ucan/",
        "https://tonk.example.evil/ucan/",
        "not a url",
    ] {
        assert_eq!(
            mount_address(Some(foreign), configured),
            configured,
            "{foreign}"
        );
    }
    assert_eq!(mount_address(None, configured), configured);
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
    let first = token(&agent, &home()).await;
    let second = token(&agent, &home()).await;
    let key = SessionKey::derive("test");
    let tickets = crate::router::hosted::view::TicketKey::derive("test");
    let endpoint = endpoint(&key, &tickets, &Unreachable);
    let session = connect(&endpoint, &first).await;

    let list = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
    let (status, body) = answer(
        endpoint
            .serve(rpc_request(Some(&first), Some(&session), list.clone()))
            .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // In catalog order, which is not this list's.
    let mut names: Vec<&str> = body["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "tonk_evaluate",
            "tonk_guide",
            "tonk_list_spaces",
            "tonk_query"
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
    let mut access = own(&agent.did());
    access.spaces.insert(
        space.did().to_string(),
        SpaceAccess {
            read: true,
            grants: vec!["bafy-read".into()],
            ..SpaceAccess::default()
        },
    );
    let writers = Granting {
        agent: agent.did(),
        access,
    };
    let key = SessionKey::derive("test");
    let tickets = crate::router::hosted::view::TicketKey::derive("test");
    let endpoint = endpoint(&key, &tickets, &writers);
    let bearer = token(&agent, &home()).await;
    let session = connect(&endpoint, &bearer).await;

    // `Granting` panics on any call but the agent space's grants, so the
    // refusal reached no writer of the space.
    let (text, error) = call_tool(
        &endpoint,
        &bearer,
        &session,
        "tonk_evaluate",
        json!({ "space": space.did().to_string(), "document": "x!: {}" }),
    )
    .await;
    assert!(error);
    assert!(text.contains("only read"), "{text}");
}

#[cfg(all(feature = "integration-tests", not(target_arch = "wasm32")))]
mod against_a_service {
    //! The endpoint over real space writers and a native access service:
    //! the MCP smoke test. In the Host Worker the same endpoint runs over
    //! Durable Objects, each holding one of these writers.
    //!
    //! Grants live in the agent space, retained there the way the
    //! approval retains them; the tokens carry only the powerline.

    use std::collections::HashMap;
    use std::sync::Arc;

    use dialog_common::helpers::Provisionable as _;
    use tonk_access_service::helpers::AccessServiceAddress;

    use super::super::super::access::DirectoryEntry;
    use super::super::super::{SpaceWriter, WriterSession};
    use super::*;

    /// An address on another origin than the configured service.
    const FOREIGN: &str = "https://elsewhere.example/ucan/";

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
        /// Each space a writer opened for, and where it mounted it.
        mounts: tokio::sync::Mutex<Vec<(Did, String)>>,
    }

    impl NativeWriters {
        async fn writer(
            &self,
            session: &WriterSession,
            space: &Did,
            remote: &str,
        ) -> Arc<SpaceWriter> {
            let mut open = self.open.lock().await;
            let name = format!(
                "{space}/{}",
                blake3::hash(&*super::super::super::writer_seed(
                    &session.seed,
                    &session.mcp_session
                ))
            );
            match open.get(&name) {
                Some(writer) => writer.clone(),
                None => {
                    let mount = url::Url::parse(remote).unwrap();
                    let writer = Arc::new(
                        SpaceWriter::open(&scratch_root(), session, space, &mount)
                            .await
                            .unwrap(),
                    );
                    self.mounts
                        .lock()
                        .await
                        .push((space.clone(), remote.to_owned()));
                    open.insert(name, writer.clone());
                    writer
                }
            }
        }
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
            let session = verified.writer_session(mcp_session);
            // As the Host Worker does: the directory's address only on the
            // configured service's origin.
            let named = request
                .headers()
                .get(ADDRESS_HEADER)
                .and_then(|address| address.to_str().ok());
            let remote = mount_address(named, self.remote.as_str());
            let writer = self.writer(&session, space, &remote).await;
            let (parts, body) = request.into_parts();
            if parts.uri.path() == ACCESS_ROUTE {
                return match writer.agent_access().await {
                    Ok(access) => rpc(StatusCode::OK, serde_json::to_value(access).unwrap()),
                    Err(error) => plain(StatusCode::BAD_GATEWAY, &error.to_string()),
                };
            }
            let grants: Vec<String> = parts
                .headers
                .get(GRANTS_HEADER)
                .and_then(|grants| grants.to_str().ok())
                .map(|grants| grants.split(',').map(str::to_owned).collect())
                .unwrap_or_default();
            if let Err(error) = writer.admit(&grants).await {
                return plain(StatusCode::BAD_GATEWAY, &error.to_string());
            }
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

    fn scratch_root() -> String {
        std::env::temp_dir()
            .join(format!("mcp-test-{}", rand::random::<u64>()))
            .to_string_lossy()
            .into_owned()
    }

    /// The session of a space's own key, as a writer of that space acts
    /// with: what the space's owner writes through here.
    async fn owner(space: &Ed25519Signer, remote: &url::Url, name: &str) -> WriterSession {
        let seed = Zeroizing::new(rand::random::<[u8; 32]>());
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
            mcp_session: name.to_owned(),
            home: remote.clone(),
        }
    }

    /// Write `documents` into `space` as its owner.
    async fn seed(space: &Ed25519Signer, remote: &url::Url, documents: &[String]) {
        let session = owner(space, remote, "seed").await;
        let writer = SpaceWriter::open(&scratch_root(), &session, &space.did(), remote)
            .await
            .unwrap();
        for document in documents {
            let response = writer
                .write(
                    Request::builder()
                        .method(Method::POST)
                        .uri("/evaluate?transact=true")
                        .header(header::CONTENT_TYPE, "text/plain")
                        .body(Body::from(document.clone()))
                        .unwrap(),
                )
                .await;
            assert!(
                response.status().is_success(),
                "seed: {}",
                response.status()
            );
        }
        writer.close().await;
    }

    /// Grant `agent` `verbs` on `space`, named `name`, as the approval
    /// does: the chains retained in the agent space with a directory
    /// entry, through a writer of the agent space of its own.
    async fn approve(
        agent: &Ed25519Signer,
        remote: &url::Url,
        space: &Ed25519Signer,
        verbs: &[&str],
        name: Option<&str>,
        address: &str,
    ) {
        let chains = grants(space, &agent.did(), verbs, None).await;
        let session = owner(agent, remote, "approval").await;
        let writer = SpaceWriter::open(&scratch_root(), &session, &agent.did(), remote)
            .await
            .unwrap();
        writer
            .approve(
                &chains,
                &[DirectoryEntry {
                    space: space.did(),
                    name: name.map(str::to_owned),
                    address: Some(address.to_owned()),
                }],
            )
            .await
            .unwrap();
        writer.close().await;
    }

    struct Client<'a> {
        endpoint: &'a McpEndpoint<'a, NativeWriters>,
        bearer: String,
        session: String,
    }

    impl<'a> Client<'a> {
        async fn connect(endpoint: &'a McpEndpoint<'a, NativeWriters>, bearer: String) -> Self {
            let session = super::connect(endpoint, &bearer).await;
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
            let (text, error) =
                call_tool(self.endpoint, &self.bearer, &self.session, name, arguments).await;
            let value = serde_json::from_str(&text).unwrap_or(json!(text));
            (value, error)
        }
    }

    #[dialog_common::test]
    async fn grants_in_the_agent_space_open_its_spaces() {
        let service = AccessServiceAddress::start(Default::default())
            .await
            .unwrap();
        let agent = Ed25519Signer::import(&rand::random::<[u8; 32]>())
            .await
            .unwrap();
        let notes = Ed25519Signer::generate().await.unwrap();
        let shelf = Ed25519Signer::generate().await.unwrap();
        for subject in [agent.did(), notes.did(), shelf.did()] {
            service
                .address
                .provision_subject(subject.as_ref())
                .await
                .unwrap();
        }
        let remote = url::Url::parse(&service.address.ucan_endpoint()).unwrap();
        let run: u64 = rand::random();
        let note = |label: &str| {
            format!("mcp-note!:\n  this: urn:mcp:{run}:{label}\n  title: \"note {label}\"\n")
        };
        let find =
            |label: &str| format!("mcp.note:\n  this: urn:mcp:{run}:{label}\n  title: ?title\n");
        // The shelf holds a note before the agent is let in.
        seed(&shelf, &remote, &[SCHEMA.to_owned(), note("shelved")]).await;
        approve(
            &agent,
            &remote,
            &notes,
            &["get", "put"],
            Some("Notes"),
            remote.as_str(),
        )
        .await;
        // The shelf's entry names a foreign service: its writer must still
        // mount it at the configured one.
        approve(&agent, &remote, &shelf, &["get"], None, FOREIGN).await;

        let writers = NativeWriters {
            remote: remote.clone(),
            open: Default::default(),
            mounts: Default::default(),
        };
        let key = SessionKey::derive("test");
        let tickets = crate::router::hosted::view::TicketKey::derive("test");
        let endpoint = endpoint(&key, &tickets, &writers);
        let client = Client::connect(&endpoint, token(&agent, &remote).await).await;

        // The list is read from the agent space, names and all.
        let (spaces, error) = client.call("tonk_list_spaces", json!({})).await;
        assert!(!error, "{spaces}");
        let listed = spaces["spaces"].as_array().unwrap();
        assert_eq!(listed.len(), 3, "{spaces}");
        assert_eq!(listed[0]["subject"], json!(agent.did().to_string()));
        assert_eq!(listed[0]["home"], json!(true));
        let entry = |did: &Did| {
            listed
                .iter()
                .find(|entry| entry["subject"] == json!(did.to_string()))
                .unwrap()
                .clone()
        };
        assert_eq!(
            entry(&notes.did()),
            json!({ "subject": notes.did().to_string(), "access": "read-write", "name": "Notes" })
        );
        assert_eq!(
            entry(&shelf.did()),
            json!({ "subject": shelf.did().to_string(), "access": "read" })
        );

        // Read and write: the schema and a note land, as the agent.
        let subject = notes.did().to_string();
        for document in [SCHEMA.to_owned(), note("written")] {
            let (result, error) = client
                .call(
                    "tonk_evaluate",
                    json!({ "space": subject, "document": document }),
                )
                .await;
            assert!(!error, "evaluate: {result}");
        }
        let (found, error) = client
            .call(
                "tonk_query",
                json!({ "space": subject, "document": find("written") }),
            )
            .await;
        assert!(!error, "query: {found}");
        assert!(found.to_string().contains("note written"), "{found}");

        // Read only: the shelf's note is seen, an evaluate is refused and
        // writes nothing.
        let shelved = shelf.did().to_string();
        let (found, error) = client
            .call(
                "tonk_query",
                json!({ "space": shelved, "document": find("shelved") }),
            )
            .await;
        assert!(!error, "query: {found}");
        assert!(found.to_string().contains("note shelved"), "{found}");
        let (refused, error) = client
            .call(
                "tonk_evaluate",
                json!({ "space": shelved, "document": note("refused") }),
            )
            .await;
        assert!(error, "a read grant evaluated: {refused}");
        assert!(refused.to_string().contains("only read"), "{refused}");
        let (after, error) = client
            .call(
                "tonk_query",
                json!({ "space": shelved, "document": find("refused") }),
            )
            .await;
        assert!(!error, "{after}");
        assert!(!after.to_string().contains("note refused"), "{after}");

        // Granted put while the shelf's writer is warm: the next evaluate
        // passes the front, and the writer pulls the new grant to land it.
        approve(&agent, &remote, &shelf, &["put"], None, FOREIGN).await;
        let (result, error) = client
            .call(
                "tonk_evaluate",
                json!({ "space": shelved, "document": note("granted") }),
            )
            .await;
        assert!(!error, "evaluate after the put grant: {result}");

        // A space the agent space holds no grant for.
        let elsewhere = Ed25519Signer::generate().await.unwrap().did().to_string();
        let (refused, error) = client
            .call(
                "tonk_query",
                json!({ "space": elsewhere, "document": find("written") }),
            )
            .await;
        assert!(error);
        assert!(refused.to_string().contains("has no access"), "{refused}");

        let mounts = writers.mounts.lock().await.clone();
        assert!(
            mounts.contains(&(shelf.did(), remote.to_string())),
            "the shelf mounted at the configured service: {mounts:?}"
        );
        assert!(
            mounts.iter().all(|(_, mounted)| mounted == remote.as_str()),
            "nothing mounted at the foreign address: {mounts:?}"
        );

        let open: Vec<Arc<SpaceWriter>> =
            writers.open.lock().await.drain().map(|(_, w)| w).collect();
        drop(open);
        service.stop().await.unwrap();
    }
}
