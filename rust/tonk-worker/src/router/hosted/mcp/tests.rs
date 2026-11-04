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
    home: Option<&url::Url>,
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
            let leaf = match home {
                Some(home) => leaf.meta(tonk_invite::home_address_meta(home)),
                None => leaf,
            };
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
    fn ignore(_: &ToolEvent) {}
    recording(key, tickets, writers, &ignore)
}

/// [`endpoint`], handing each tool call's event to `events`.
fn recording<'a, W: Writers>(
    key: &'a SessionKey,
    tickets: &'a crate::router::hosted::view::TicketKey,
    writers: &'a W,
    events: telemetry::Sink<'a>,
) -> McpEndpoint<'a, W> {
    McpEndpoint {
        origin: ORIGIN,
        key,
        tickets,
        writers,
        events,
    }
}

/// Events an endpoint handed its sink.
#[derive(Default)]
struct Events(std::cell::RefCell<Vec<ToolEvent>>);

impl Events {
    fn sink(&self) -> impl Fn(&ToolEvent) + '_ {
        |event| self.0.borrow_mut().push(event.clone())
    }

    fn only(&self) -> ToolEvent {
        let events = self.0.borrow();
        assert_eq!(events.len(), 1, "{events:?}");
        events[0].clone()
    }

    fn last(&self) -> ToolEvent {
        self.0.borrow().last().cloned().expect("an event")
    }
}

/// A `tools/call` request.
fn tool_request(bearer: &str, session: &str, name: &str, arguments: Value) -> Request<Body> {
    rpc_request(
        Some(bearer),
        Some(session),
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments },
        }),
    )
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
async fn the_manuals_it_names_are_resources_it_serves() {
    let listed = guide_resources();
    assert_eq!(listed.len(), guide::TOPICS.len());
    for resource in &listed {
        let uri = resource["uri"].as_str().unwrap();
        let read = read_guide(uri).unwrap();
        assert!(!read["contents"][0]["text"].as_str().unwrap().is_empty());
    }
    assert!(read_guide("tonk://guide/missing").is_none());
    let said = instructions("https://tonk.example/");
    assert!(said.starts_with("# Working with Tonk"));
    assert!(said.contains("Tonk at https://tonk.example."));
    assert!(said.contains("`tonk_open_space` shows a route"));
    assert!(!said.contains("{origin}"));
    let agent = read_guide("tonk://guide/agent").unwrap();
    assert_eq!(agent["contents"][0]["text"], HOSTED);
    let answer = json!({ "revision_before": [1, 2], "revision_after": [3], "matches": [] });
    assert_eq!(without_revisions(answer), json!({ "matches": [] }));
}

#[dialog_common::test]
async fn a_worker_answer_reads_as_notation_with_its_next_page() {
    let body = json!({
        "revision_before": null,
        "revision_after": null,
        "matches_before": [],
        "matches_after": [{
            "label": "person",
            "results": [{ "this": "did:key:zHj", "fields": { "name": "Alice" }, "transient": false }],
        }],
        "commits": { "claims": 0, "entities": {} },
        "page": { "rows": 1, "next": "abc" },
    })
    .to_string();
    let text = notation(body.as_bytes(), false).unwrap();
    assert!(text.contains("Alice"), "{text}");
    assert!(!text.trim_start().starts_with('{'), "{text}");
    assert!(text.ends_with("cursor: \"abc\"\n"), "{text}");
    assert!(notation(b"not json", false).is_none());
}

#[dialog_common::test]
async fn an_empty_write_says_it_wrote_nothing() {
    let answer = |claims: u64| {
        json!({
            "revision_before": null,
            "revision_after": null,
            "matches_before": [],
            "matches_after": [{
                "label": "person",
                "results": [
                    { "this": "did:key:zHj", "fields": { "name": "Alice" }, "transient": false },
                    { "this": "did:key:zHk", "fields": { "name": "Bob" }, "transient": false },
                ],
            }],
            "commits": { "claims": claims, "entities": {} },
        })
        .to_string()
    };
    let read = notation(answer(0).as_bytes(), false).unwrap();
    assert!(read.contains("Alice") && read.contains("Bob"), "{read}");
    assert!(!read.contains("Nothing was written"), "{read}");

    let pasted = notation(answer(0).as_bytes(), true).unwrap();
    assert!(pasted.contains("Nothing was written"), "{pasted}");
    let wrote = notation(answer(2).as_bytes(), true).unwrap();
    assert!(!wrote.contains("Nothing was written"), "{wrote}");
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
    let carried = grants(&space, &agent.did(), &["get", "put"], None, None).await;
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
    assert_eq!(spaces[0]["access"], json!("write"));
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
        json!({ "subject": plans.to_string(), "access": "write" })
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
fn a_space_mounts_at_its_recorded_address() {
    let configured = "https://tonk.example/ucan/";
    for named in [
        "https://tonk.example/ucan/",
        "https://tonk.example/other/ucan/",
        "https://tonk.network/ucan/",
        "https://tonk.example:8443/ucan/",
        "http://127.0.0.1:8787/ucan/",
        "http://localhost:8787/ucan/",
    ] {
        assert_eq!(mount_address(Some(named), configured), named, "{named}");
    }
    for unusable in [
        "http://tonk.example/ucan/",
        "file:///etc/passwd",
        "not a url",
    ] {
        assert_eq!(
            mount_address(Some(unusable), configured),
            configured,
            "{unusable}"
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
            "tonk_join_accept",
            "tonk_join_space",
            "tonk_list_spaces",
            "tonk_open_space",
            "tonk_outline",
            "tonk_proposal_apply",
            "tonk_proposal_dismiss",
            "tonk_query",
            "tonk_undo",
            "tonk_view_access"
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

#[dialog_common::test]
async fn undo_is_listed_for_the_app_alone() {
    let agent = Ed25519Signer::generate().await.unwrap();
    let key = SessionKey::derive("test");
    let tickets = crate::router::hosted::view::TicketKey::derive("test");
    let endpoint = endpoint(&key, &tickets, &Unreachable);
    let bearer = token(&agent, &home()).await;
    let session = connect(&endpoint, &bearer).await;
    let list = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
    let (status, body) = answer(
        endpoint
            .serve(rpc_request(Some(&bearer), Some(&session), list))
            .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let undo = body["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == json!(UNDO))
        .cloned()
        .expect("tonk_undo is listed");
    assert_eq!(undo["_meta"]["ui"]["visibility"], json!(["app"]));
    assert_eq!(
        undo["inputSchema"]["required"],
        json!(["space", "revision"])
    );
}

#[dialog_common::test]
async fn a_write_carries_its_change_for_the_card_and_a_read_does_not() {
    let space = Ed25519Signer::generate().await.unwrap().did();
    let before = json!({ "tree": "a" });
    let revision = |answer: &Value| undo::change_meta(&space, answer);
    // Nothing committed: no change to undo.
    let unchanged = json!({
        "revision_before": before,
        "revision_after": before,
        "commits": { "claims": 0, "entities": {} },
    });
    assert!(revision(&unchanged).is_none());
    // An answer whose revision does not decode carries nothing rather
    // than a revision the undo could not find.
    let unreadable = json!({
        "revision_before": null,
        "revision_after": { "tree": "b" },
        "commits": { "claims": 1, "entities": {} },
    });
    assert!(revision(&unreadable).is_none());
}

#[dialog_common::test]
async fn undo_on_a_read_grant_is_refused_before_any_writer() {
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

    let (text, error) = call_tool(
        &endpoint,
        &bearer,
        &session,
        UNDO,
        json!({ "space": space.did().to_string(), "revision": "abc" }),
    )
    .await;
    assert!(error);
    assert!(text.contains("only read"), "{text}");
}

/// Call a tool: its whole result.
async fn call_result<W: Writers>(
    endpoint: &McpEndpoint<'_, W>,
    bearer: &str,
    session: &str,
    name: &str,
    arguments: Value,
) -> Value {
    let call = json!({
        "jsonrpc": "2.0",
        "id": 5,
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
    body["result"].clone()
}

#[dialog_common::test]
async fn evaluate_offers_to_confirm_first_and_draws_its_result_in_the_card() {
    let evaluate = tools()
        .into_iter()
        .find(|tool| tool["name"] == "tonk_evaluate")
        .unwrap();
    assert_eq!(
        evaluate["inputSchema"]["properties"]["confirm"]["type"],
        "boolean"
    );
    assert_eq!(
        evaluate["_meta"]["ui"]["resourceUri"],
        super::super::card::CARD_URI
    );
    assert_eq!(evaluate["_meta"]["openai/widgetAccessible"], true);
    for tool in super::super::propose::tools() {
        assert_eq!(tool["_meta"]["ui"]["visibility"], json!(["app"]), "{tool}");
        assert_eq!(tool["_meta"]["openai/visibility"], "private", "{tool}");
    }
}

#[dialog_common::test]
async fn a_refused_write_is_a_tell_with_a_role_only_where_the_space_is_open() {
    let agent = Ed25519Signer::generate().await.unwrap();
    let shelf = Ed25519Signer::generate().await.unwrap();
    let elsewhere = Ed25519Signer::generate().await.unwrap();
    let mut access = own(&agent.did());
    access.spaces.insert(
        shelf.did().to_string(),
        SpaceAccess {
            read: true,
            name: Some("Shelf".into()),
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

    for (space, confirm) in [(&shelf, false), (&shelf, true)] {
        let result = call_result(
            &endpoint,
            &bearer,
            &session,
            "tonk_evaluate",
            json!({ "space": space.did().to_string(), "document": "x!: {}", "confirm": confirm }),
        )
        .await;
        assert_eq!(result["isError"], true);
        let card = &result["structuredContent"]["card"];
        assert_eq!(card["kind"], "tell", "{result}");
        assert_eq!(card["label"], "read only");
        assert_eq!(card["role"], "read");
        assert_eq!(card["space"]["name"], "Shelf");
        assert_eq!(card["agent"]["name"], "Claude");
    }

    let result = call_result(
        &endpoint,
        &bearer,
        &session,
        "tonk_evaluate",
        json!({ "space": elsewhere.did().to_string(), "document": "x!: {}" }),
    )
    .await;
    let card = &result["structuredContent"]["card"];
    assert_eq!(card["label"], "no access", "{result}");
    assert!(card.get("role").is_none(), "{card}");

    // An answer to a proposal is checked the same way, before any writer.
    let result = call_result(
        &endpoint,
        &bearer,
        &session,
        "tonk_proposal_apply",
        json!({ "space": shelf.did().to_string(), "id": "held" }),
    )
    .await;
    assert_eq!(result["isError"], true);
    assert_eq!(result["structuredContent"]["card"]["label"], "read only");
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
            // As the Host Worker does: the grant's home, or the configured
            // service.
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
                    // As the Host Worker does: an upstream refusal keeps
                    // its status and code.
                    Err(crate::TonkWorkerError::Upstream {
                        status,
                        code: Some(code),
                        message,
                    }) => rpc(
                        StatusCode::from_u16(status).unwrap(),
                        json!({ "error": message, "code": code }),
                    ),
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
            if parts.uri.path() == UNDO_ROUTE {
                let body = body.collect().await.unwrap().to_bytes();
                let body: Value = serde_json::from_slice(&body).unwrap();
                return match writer.undo(body["revision"].as_str().unwrap()).await {
                    Ok(report) => rpc(StatusCode::OK, serde_json::to_value(report).unwrap()),
                    Err(error) => plain(StatusCode::BAD_GATEWAY, &error),
                };
            }
            let routed = |uri: &str, body: Body| {
                Request::builder()
                    .method(Method::POST)
                    .uri(uri)
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(body)
                    .unwrap()
            };
            let id = |body: &[u8]| {
                serde_json::from_slice::<Value>(body).unwrap()["id"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            };
            match parts.uri.path() {
                "/api/query" => writer.read(routed("/evaluate?transact=false", body)).await,
                "/api/evaluate" => writer.write(routed("/evaluate?transact=true", body)).await,
                path if path == crate::router::hosted::propose::PROPOSE_ROUTE => {
                    let body = body.collect().await.unwrap().to_bytes();
                    writer
                        .propose(String::from_utf8(body.to_vec()).unwrap())
                        .await
                }
                path if path == crate::router::hosted::propose::APPLY_ROUTE => {
                    let body = body.collect().await.unwrap().to_bytes();
                    writer.apply(&id(&body)).await
                }
                path if path == crate::router::hosted::propose::DISMISS_ROUTE => {
                    let body = body.collect().await.unwrap().to_bytes();
                    writer.dismiss(&id(&body))
                }
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
    /// does: the chains, carrying `home` in their signed meta, retained in
    /// the agent space with a directory entry naming `address`, through a
    /// writer of the agent space of its own.
    async fn approve(
        agent: &Ed25519Signer,
        remote: &url::Url,
        space: &Ed25519Signer,
        verbs: &[&str],
        name: Option<&str>,
        home: Option<&url::Url>,
        address: &str,
    ) -> Vec<DelegationChain> {
        let chains = grants(space, &agent.did(), verbs, None, home).await;
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
        chains
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

        /// Call a tool: its whole result.
        async fn result(&self, name: &str, arguments: Value) -> Value {
            let call = json!({
                "jsonrpc": "2.0",
                "id": 4,
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
            body["result"].clone()
        }

        /// Call a tool: its result and whether it is an error.
        async fn call(&self, name: &str, arguments: Value) -> (Value, bool) {
            let (text, error) =
                call_tool(self.endpoint, &self.bearer, &self.session, name, arguments).await;
            let value = serde_json::from_str(&text).unwrap_or(json!(text));
            (value, error)
        }
    }

    /// How long after a revocation the agent may still be answered. The
    /// same budgets as the hosted-agent e2e (`REVOKE_BUDGET` and
    /// `LINK_REVOKE_BUDGET` in `tests/e2e/hosted-agent/agent.test.mjs`):
    /// the two run one scenario and must agree.
    const REVOKE_BUDGET: std::time::Duration = std::time::Duration::from_secs(30);
    const LINK_REVOKE_BUDGET: std::time::Duration = std::time::Duration::from_secs(60);

    /// How the person takes a grant away.
    #[derive(Clone, Copy, Debug)]
    enum Revocation {
        /// A grant this profile issued: revoked at the access service.
        AtTheService,
        /// A grant joined from a share link: dropped from the agent space
        /// only, as `agents::revoke` does.
        FromTheAgentSpace,
    }

    /// Call a tool built by `arguments` (given the attempt's number) until
    /// the agent is refused, from `since` for at most `budget`: how long
    /// that took, and the refusal's event.
    async fn refused_within(
        client: &Client<'_>,
        events: &Events,
        name: &str,
        arguments: impl Fn(usize) -> Value,
        since: std::time::Instant,
        budget: std::time::Duration,
    ) -> (std::time::Duration, ToolEvent) {
        for attempt in 0.. {
            let (result, error) = client.call(name, arguments(attempt)).await;
            let elapsed = since.elapsed();
            // Past the budget fails whether or not this call was refused:
            // a refusal that arrives late is a late revocation.
            assert!(
                elapsed <= budget,
                "{name} was {} {elapsed:?} after the revocation (budget {budget:?}): {result}",
                if error {
                    "refused only"
                } else {
                    "still answered"
                },
            );
            if error {
                println!("{name} refused: {result}");
                return (elapsed, events.last());
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        unreachable!()
    }

    /// The person takes the agent's read and write grant away, `how`, and
    /// both its reads and its writes on the space are refused within
    /// `budget`: the scenario the hosted-agent e2e runs on a preview.
    async fn revoking_refuses_reads_and_writes(how: Revocation, budget: std::time::Duration) {
        let service = AccessServiceAddress::start(Default::default())
            .await
            .unwrap();
        let agent = Ed25519Signer::import(&rand::random::<[u8; 32]>())
            .await
            .unwrap();
        let notes = Ed25519Signer::generate().await.unwrap();
        for subject in [agent.did(), notes.did()] {
            service
                .address
                .provision_subject(subject.as_ref())
                .await
                .unwrap();
        }
        let remote = url::Url::parse(&service.address.ucan_endpoint()).unwrap();
        let chains = approve(
            &agent,
            &remote,
            &notes,
            &["get", "put"],
            Some("Notes"),
            None,
            remote.as_str(),
        )
        .await;

        let writers = NativeWriters {
            remote: remote.clone(),
            open: Default::default(),
            mounts: Default::default(),
        };
        let events = Events::default();
        let sink = events.sink();
        let key = SessionKey::derive("test");
        let tickets = crate::router::hosted::view::TicketKey::derive("test");
        let endpoint = recording(&key, &tickets, &writers, &sink);
        let client = Client::connect(&endpoint, token(&agent, &remote).await).await;
        let subject = notes.did().to_string();
        let read = |_: usize| json!({ "space": subject, "document": "concept:\n" });
        let run: u64 = rand::random();
        let write = |attempt: usize| {
            json!({
                "space": subject,
                "document": format!(
                    "mcp-note!:\n  this: urn:mcp:{run}:{attempt}\n  title: \"probe {attempt}\"\n"
                ),
            })
        };
        // Answered before, so a refusal after is the revocation's.
        let (result, error) = client
            .call(
                "tonk_evaluate",
                json!({ "space": subject, "document": SCHEMA }),
            )
            .await;
        assert!(!error, "evaluate before the revocation: {result}");
        let (result, error) = client.call("tonk_query", read(0)).await;
        assert!(!error, "query before the revocation: {result}");

        let since = std::time::Instant::now();
        match how {
            Revocation::AtTheService => {
                for chain in &chains {
                    let leaf = crate::router::revoke_invite::leaf_cid(chain).unwrap();
                    let revocation = tonk_identity::revocation::mint_root_revocation(
                        notes.clone(),
                        chain,
                        &leaf,
                    )
                    .await
                    .unwrap();
                    crate::router::http::post_cbor(&remote, &revocation)
                        .await
                        .unwrap();
                }
            }
            Revocation::FromTheAgentSpace => {
                let session = owner(&agent, &remote, "withdrawal").await;
                let writer = SpaceWriter::open(&scratch_root(), &session, &agent.did(), &remote)
                    .await
                    .unwrap();
                writer.withdraw(&chains).await.unwrap();
                writer.close().await;
            }
        }

        for (name, arguments) in [
            ("tonk_query", &read as &dyn Fn(usize) -> Value),
            ("tonk_evaluate", &write),
        ] {
            let (elapsed, refused) =
                refused_within(&client, &events, name, arguments, since, budget).await;
            println!("{how:?}: {name} refused {elapsed:?} after the revocation");
            assert_eq!(refused.outcome, telemetry::Outcome::Refused, "{refused:?}");
            assert!(
                matches!(
                    refused.kind.as_deref(),
                    Some("credential_revoked" | "access_denied" | "no_access")
                ),
                "{name} refused for another reason: {refused:?}"
            );
        }
    }

    #[dialog_common::test]
    async fn revoking_an_approved_grant_refuses_reads_and_writes_within_the_budget() {
        revoking_refuses_reads_and_writes(Revocation::AtTheService, REVOKE_BUDGET).await;
    }

    #[dialog_common::test]
    async fn revoking_a_share_link_grant_refuses_reads_and_writes_within_the_budget() {
        revoking_refuses_reads_and_writes(Revocation::FromTheAgentSpace, LINK_REVOKE_BUDGET).await;
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
        // Notes lives on another service, as a space homed on another
        // deployment does: its grant says so.
        let other = AccessServiceAddress::start(Default::default())
            .await
            .unwrap();
        other
            .address
            .provision_subject(notes.did().as_ref())
            .await
            .unwrap();
        let notes_home = url::Url::parse(&other.address.ucan_endpoint()).unwrap();
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
            Some(&notes_home),
            FOREIGN,
        )
        .await;
        // The shelf's grant names no home and its directory entry names a
        // foreign service, which the agent could have written: its writer
        // mounts it at the configured service.
        approve(&agent, &remote, &shelf, &["get"], None, None, FOREIGN).await;

        let writers = NativeWriters {
            remote: remote.clone(),
            open: Default::default(),
            mounts: Default::default(),
        };
        let key = SessionKey::derive("test");
        let tickets = crate::router::hosted::view::TicketKey::derive("test");
        let events = Events::default();
        let sink = events.sink();
        let endpoint = recording(&key, &tickets, &writers, &sink);
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
            json!({ "subject": notes.did().to_string(), "access": "write", "name": "Notes" })
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
        // The write's own steps, as the writer timed them, reach the event.
        let written = events.last();
        assert_eq!(written.outcome, telemetry::Outcome::Ok);
        for step in ["writer.lock", "writer.pull", "writer.eval", "writer.push"] {
            assert!(written.step(step).is_some(), "{step}: {:?}", written.steps);
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
        approve(&agent, &remote, &shelf, &["put"], None, None, FOREIGN).await;
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
            mounts.contains(&(notes.did(), notes_home.to_string())),
            "notes mounted at its grant's home: {mounts:?}"
        );
        assert!(
            mounts.contains(&(shelf.did(), remote.to_string())),
            "the shelf mounted at the configured service: {mounts:?}"
        );
        assert!(
            mounts.iter().all(|(_, mounted)| mounted != FOREIGN),
            "nothing mounted at the directory's address: {mounts:?}"
        );

        let open: Vec<Arc<SpaceWriter>> =
            writers.open.lock().await.drain().map(|(_, w)| w).collect();
        drop(open);
        service.stop().await.unwrap();
        other.stop().await.unwrap();
    }

    /// A card undoes a write with the revision its result carried, through
    /// the app-only tool; the space then reads as before the write, the
    /// outcome is recorded in it, and a second undo is refused.
    #[dialog_common::test]
    async fn a_card_undoes_a_write_with_the_revision_it_carried() {
        let service = AccessServiceAddress::start(Default::default())
            .await
            .unwrap();
        let agent = Ed25519Signer::import(&rand::random::<[u8; 32]>())
            .await
            .unwrap();
        service
            .address
            .provision_subject(agent.did().as_ref())
            .await
            .unwrap();
        let remote = url::Url::parse(&service.address.ucan_endpoint()).unwrap();
        let writers = NativeWriters {
            remote: remote.clone(),
            open: Default::default(),
            mounts: Default::default(),
        };
        let key = SessionKey::derive("test");
        let tickets = crate::router::hosted::view::TicketKey::derive("test");
        let endpoint = endpoint(&key, &tickets, &writers);
        let client = Client::connect(&endpoint, token(&agent, &remote).await).await;
        let space = agent.did().to_string();
        let run: u64 = rand::random();
        let note = format!("mcp-note!:\n  this: urn:mcp:{run}:undo\n  title: \"undo me\"\n");
        let find = format!("mcp.note:\n  this: urn:mcp:{run}:undo\n  title: ?title\n");

        let schema = client
            .result(
                "tonk_evaluate",
                json!({ "space": space, "document": SCHEMA }),
            )
            .await;
        assert_eq!(schema["isError"], json!(false), "{schema}");
        let written = client
            .result("tonk_evaluate", json!({ "space": space, "document": note }))
            .await;
        assert_eq!(written["isError"], json!(false), "{written}");
        let change = &written["_meta"][CHANGE_META];
        assert_eq!(change["space"], json!(space), "{written}");
        let revision = change["revision"].as_str().expect("a revision").to_owned();
        // The model's text and structured content carry no revision.
        assert!(!written["content"].to_string().contains(&revision));
        assert!(!written["structuredContent"].to_string().contains(&revision));

        let undone = client
            .result(UNDO, json!({ "space": space, "revision": revision }))
            .await;
        assert_eq!(undone["isError"], json!(false), "{undone}");
        assert_eq!(
            undone["structuredContent"]["status"],
            json!("undone"),
            "{undone}"
        );
        assert!(
            undone["structuredContent"]["committed"].is_string(),
            "{undone}"
        );

        let (found, error) = client
            .call("tonk_query", json!({ "space": space, "document": find }))
            .await;
        assert!(!error, "{found}");
        assert!(!found.to_string().contains("undo me"), "{found}");
        let outcome = format!("xyz.tonk.undo:\n  this: tonk:undo/{revision}\n  status: ?status\n");
        let (recorded, error) = client
            .call("tonk_query", json!({ "space": space, "document": outcome }))
            .await;
        assert!(!error, "{recorded}");
        assert!(recorded.to_string().contains("undone"), "{recorded}");

        let again = client
            .result(UNDO, json!({ "space": space, "revision": revision }))
            .await;
        assert_eq!(
            again["structuredContent"]["status"],
            json!("refused"),
            "{again}"
        );
        assert!(
            again["structuredContent"]["reason"]
                .as_str()
                .unwrap()
                .contains("undone already"),
            "{again}"
        );

        let open: Vec<Arc<SpaceWriter>> =
            writers.open.lock().await.drain().map(|(_, w)| w).collect();
        drop(open);
        service.stop().await.unwrap();
    }

    #[dialog_common::test]
    async fn a_proposal_is_applied_once_and_only_in_its_own_session() {
        let service = AccessServiceAddress::start(Default::default())
            .await
            .unwrap();
        let agent = Ed25519Signer::import(&rand::random::<[u8; 32]>())
            .await
            .unwrap();
        service
            .address
            .provision_subject(agent.did().as_ref())
            .await
            .unwrap();
        let remote = url::Url::parse(&service.address.ucan_endpoint()).unwrap();
        let writers = NativeWriters {
            remote: remote.clone(),
            open: Default::default(),
            mounts: Default::default(),
        };
        let key = SessionKey::derive("test");
        let tickets = crate::router::hosted::view::TicketKey::derive("test");
        let endpoint = endpoint(&key, &tickets, &writers);
        let bearer = token(&agent, &remote).await;
        let client = Client::connect(&endpoint, bearer.clone()).await;
        // Another MCP session of the same connection: another writer.
        let other = Client::connect(&endpoint, bearer).await;
        let space = agent.did().to_string();
        let run: u64 = rand::random();
        let note = |label: &str| {
            format!("mcp-note!:\n  this: urn:mcp:{run}:{label}\n  title: \"note {label}\"\n")
        };
        let holds = |label: &str| {
            let find = format!("mcp.note:\n  this: urn:mcp:{run}:{label}\n  title: ?title\n");
            let client = &client;
            let space = space.clone();
            let label = label.to_owned();
            async move {
                let (found, error) = client
                    .call("tonk_query", json!({ "space": space, "document": find }))
                    .await;
                assert!(!error, "{found}");
                found.to_string().contains(&format!("note {label}"))
            }
        };

        // A plain write is saved at once, and its card is the receipt.
        let wrote = client
            .result(
                "tonk_evaluate",
                json!({ "space": space, "document": SCHEMA }),
            )
            .await;
        assert_eq!(wrote["isError"], false, "{wrote}");
        let receipt = &wrote["structuredContent"]["card"];
        assert_eq!(receipt["kind"], "strip", "{wrote}");
        assert_eq!(receipt["byAgent"], true);
        assert_eq!(receipt["agent"]["did"], json!(space));

        // Proposed: an ask, and nothing written.
        let proposed = client
            .result(
                "tonk_evaluate",
                json!({ "space": space, "document": note("asked"), "confirm": true }),
            )
            .await;
        assert_eq!(proposed["isError"], false, "{proposed}");
        let ask = &proposed["structuredContent"]["card"];
        assert_eq!(ask["kind"], "ask", "{proposed}");
        assert_eq!(ask["role"], "write");
        assert_eq!(ask["changes"][0]["mark"], "+", "{ask}");
        assert_eq!(ask["changes"][0]["name"], "note asked");
        let answers = ask["answers"].as_array().unwrap();
        assert_eq!(answers[0]["tool"], "tonk_proposal_dismiss");
        assert_eq!(answers[1]["tool"], "tonk_proposal_apply");
        let arguments = answers[1]["arguments"].clone();
        assert!(
            proposed["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Nothing is written unless they apply it")
        );
        assert!(!holds("asked").await, "a proposal wrote");

        // Another session does not hold it, and leaves it held.
        let elsewhere = other.result("tonk_proposal_apply", arguments.clone()).await;
        assert_eq!(elsewhere["isError"], true, "{elsewhere}");
        assert_eq!(
            elsewhere["structuredContent"]["card"]["label"],
            "not applied"
        );
        assert!(!holds("asked").await, "another session applied it");

        // Applied once, by its own session.
        let applied = client
            .result("tonk_proposal_apply", arguments.clone())
            .await;
        assert_eq!(applied["isError"], false, "{applied}");
        let strip = &applied["structuredContent"]["card"];
        assert_eq!(strip["kind"], "strip", "{applied}");
        assert_eq!(strip["byAgent"], true);
        assert!(
            strip["say"].as_str().unwrap().starts_with("saved 1 item"),
            "{strip}"
        );
        assert!(holds("asked").await, "the applied note is missing");

        // Twice: refused, nothing more written.
        let again = client.result("tonk_proposal_apply", arguments).await;
        assert_eq!(again["isError"], true, "{again}");
        assert_eq!(again["structuredContent"]["card"]["kind"], "tell");

        // Dismissed: nothing written, and it cannot be applied after.
        let proposed = client
            .result(
                "tonk_evaluate",
                json!({ "space": space, "document": note("declined"), "confirm": true }),
            )
            .await;
        let arguments = proposed["structuredContent"]["card"]["answers"][0]["arguments"].clone();
        let dismissed = client
            .result("tonk_proposal_dismiss", arguments.clone())
            .await;
        assert_eq!(dismissed["isError"], false, "{dismissed}");
        assert_eq!(dismissed["structuredContent"]["card"]["kind"], "strip");
        let late = client.result("tonk_proposal_apply", arguments).await;
        assert_eq!(late["isError"], true, "{late}");
        assert!(!holds("declined").await, "a dismissed proposal wrote");

        // The space moves after the preview: the confirmed change is not
        // written over what the person never saw.
        let proposed = client
            .result(
                "tonk_evaluate",
                json!({ "space": space, "document": note("stale"), "confirm": true }),
            )
            .await;
        let arguments = proposed["structuredContent"]["card"]["answers"][1]["arguments"].clone();
        let (moved, error) = client
            .call(
                "tonk_evaluate",
                json!({ "space": space, "document": note("meanwhile") }),
            )
            .await;
        assert!(!error, "{moved}");
        let stale = client.result("tonk_proposal_apply", arguments).await;
        assert_eq!(stale["isError"], true, "{stale}");
        assert_eq!(
            stale["structuredContent"]["card"]["label"], "changed since",
            "{stale}"
        );
        assert!(!holds("stale").await, "a stale proposal wrote");

        // A document that writes nothing has nothing to confirm.
        let read = client
            .result(
                "tonk_evaluate",
                json!({
                    "space": space,
                    "document": format!("mcp.note:\n  this: urn:mcp:{run}:asked\n  title: ?t\n"),
                    "confirm": true,
                }),
            )
            .await;
        assert_eq!(read["isError"], true, "{read}");
        assert!(
            read["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("writes nothing")
        );

        let open: Vec<Arc<SpaceWriter>> =
            writers.open.lock().await.drain().map(|(_, w)| w).collect();
        drop(open);
        service.stop().await.unwrap();
    }
}

/// Writers that answer the agent space's grants with `access`, and a
/// space's reads and writes with `answer`; each records the trace id the
/// request carried.
struct Scripted {
    agent: Did,
    access: AgentAccess,
    answer: fn() -> Response<Body>,
    traces: std::cell::RefCell<Vec<Option<String>>>,
}

impl Writers for Scripted {
    async fn call(
        &self,
        _verified: &Verified,
        _bearer: &str,
        _mcp_session: &str,
        space: &Did,
        request: Request<Body>,
    ) -> Response<Body> {
        self.traces
            .borrow_mut()
            .push(telemetry::trace_of(request.headers()));
        if request.uri().path() == ACCESS_ROUTE {
            assert_eq!(space, &self.agent);
            let mut response = rpc(StatusCode::OK, serde_json::to_value(&self.access).unwrap());
            response
                .headers_mut()
                .insert(SERVER_TIMING, HeaderValue::from_static("pull;dur=7"));
            return response;
        }
        (self.answer)()
    }
}

/// The agent's own space and `space`, granted read and write.
fn writable(agent: &Did, space: &Did) -> AgentAccess {
    let mut access = own(agent);
    access.spaces.insert(
        space.to_string(),
        SpaceAccess {
            read: true,
            write: true,
            grants: vec!["bafy-write".into()],
            ..SpaceAccess::default()
        },
    );
    access
}

#[dialog_common::test]
async fn a_tool_call_answers_its_trace_and_the_time_of_each_step() {
    let agent = Ed25519Signer::generate().await.unwrap();
    let writers = Scripted {
        agent: agent.did(),
        access: own(&agent.did()),
        answer: || panic!("no space was named"),
        traces: Default::default(),
    };
    let events = Events::default();
    let sink = events.sink();
    let key = SessionKey::derive("test");
    let tickets = crate::router::hosted::view::TicketKey::derive("test");
    let endpoint = recording(&key, &tickets, &writers, &sink);
    let bearer = token(&agent, &home()).await;
    let session = connect(&endpoint, &bearer).await;

    let response = endpoint
        .serve(tool_request(
            &bearer,
            &session,
            "tonk_list_spaces",
            json!({}),
        ))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let trace = response.headers()[TRACE_HEADER]
        .to_str()
        .unwrap()
        .to_owned();
    let timing = response.headers()[SERVER_TIMING]
        .to_str()
        .unwrap()
        .to_owned();

    // The trace reached the writer, and comes back to the client.
    assert_eq!(*writers.traces.borrow(), vec![Some(trace.clone())]);
    for step in ["verify;", "access;", "access.pull;dur=7", "total;"] {
        assert!(timing.contains(step), "{step} missing from {timing}");
    }
    let event = events.only();
    assert_eq!(event.trace, trace);
    assert_eq!(event.tool, "tonk_list_spaces");
    assert_eq!(event.outcome, telemetry::Outcome::Ok);
    assert_eq!(event.kind, None);
    assert_eq!(event.agent, telemetry::short_did(&agent.did()));
    assert_eq!(event.step("access.pull"), Some(7.0));
    assert!(event.ms >= 0.0);
}

#[dialog_common::test]
async fn a_refusal_is_recorded_with_its_reason_and_space() {
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
    let writers = Scripted {
        agent: agent.did(),
        access,
        answer: || panic!("a refused call reached a writer"),
        traces: Default::default(),
    };
    let events = Events::default();
    let sink = events.sink();
    let key = SessionKey::derive("test");
    let tickets = crate::router::hosted::view::TicketKey::derive("test");
    let endpoint = recording(&key, &tickets, &writers, &sink);
    let bearer = token(&agent, &home()).await;
    let session = connect(&endpoint, &bearer).await;

    let (_, error) = call_tool(
        &endpoint,
        &bearer,
        &session,
        "tonk_evaluate",
        json!({ "space": space.did().to_string(), "document": "x!: {}" }),
    )
    .await;
    assert!(error);
    let event = events.only();
    assert_eq!(event.outcome, telemetry::Outcome::Refused);
    assert_eq!(event.kind.as_deref(), Some("read_only"));
    assert_eq!(event.space, Some(telemetry::short_did(&space.did())));
}

#[dialog_common::test]
async fn a_malformed_call_is_invalid_not_a_failure() {
    let agent = Ed25519Signer::generate().await.unwrap();
    let events = Events::default();
    let sink = events.sink();
    let key = SessionKey::derive("test");
    let tickets = crate::router::hosted::view::TicketKey::derive("test");
    let endpoint = recording(&key, &tickets, &Unreachable, &sink);
    let bearer = token(&agent, &home()).await;
    let session = connect(&endpoint, &bearer).await;

    let (_, error) = call_tool(
        &endpoint,
        &bearer,
        &session,
        "tonk_query",
        json!({ "document": "x: {}" }),
    )
    .await;
    assert!(error);
    let event = events.only();
    assert_eq!(event.outcome, telemetry::Outcome::Invalid);
    assert_eq!(event.kind.as_deref(), Some("no_space"));

    // A name this endpoint could not serve is not written as given.
    call_tool(&endpoint, &bearer, &session, "Not A Tool!", json!({})).await;
    assert_eq!(events.last().tool, "other");
}

#[dialog_common::test]
async fn a_writer_failure_is_recorded_with_its_code_and_steps() {
    let agent = Ed25519Signer::generate().await.unwrap();
    let space = Ed25519Signer::generate().await.unwrap();
    let writers = Scripted {
        agent: agent.did(),
        access: writable(&agent.did(), &space.did()),
        answer: || {
            let mut response = rpc(
                StatusCode::BAD_GATEWAY,
                json!({ "error": "push: 503", "code": "WRITE_NOT_LANDED" }),
            );
            response.headers_mut().insert(
                SERVER_TIMING,
                HeaderValue::from_static("lock;dur=1, pull;dur=20, eval;dur=3, push;dur=40"),
            );
            response
        },
        traces: Default::default(),
    };
    let events = Events::default();
    let sink = events.sink();
    let key = SessionKey::derive("test");
    let tickets = crate::router::hosted::view::TicketKey::derive("test");
    let endpoint = recording(&key, &tickets, &writers, &sink);
    let bearer = token(&agent, &home()).await;
    let session = connect(&endpoint, &bearer).await;

    let response = endpoint
        .serve(tool_request(
            &bearer,
            &session,
            "tonk_evaluate",
            json!({ "space": space.did().to_string(), "document": "x!: {}" }),
        ))
        .await;
    let timing = response.headers()[SERVER_TIMING]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(timing.contains("writer.push;dur=40.0"), "{timing}");
    let trace = response.headers()[TRACE_HEADER]
        .to_str()
        .unwrap()
        .to_owned();
    // The access read and the write carried the same trace.
    assert_eq!(
        *writers.traces.borrow(),
        vec![Some(trace.clone()), Some(trace)]
    );

    let event = events.only();
    assert_eq!(event.outcome, telemetry::Outcome::Error);
    assert_eq!(event.kind.as_deref(), Some("write_not_landed"));
    assert_eq!(event.step("writer.pull"), Some(20.0));
    assert_eq!(event.step("writer.push"), Some(40.0));
    assert!(event.step("writer").is_some(), "{:?}", event.steps);
}

#[dialog_common::test]
fn a_rejected_request_is_invalid_and_a_refused_one_refused() {
    assert_eq!(
        writer_outcome(StatusCode::BAD_REQUEST),
        telemetry::Outcome::Invalid
    );
    assert_eq!(
        writer_outcome(StatusCode::FORBIDDEN),
        telemetry::Outcome::Refused
    );
    assert_eq!(
        writer_outcome(StatusCode::GATEWAY_TIMEOUT),
        telemetry::Outcome::Error
    );
}

/// Writers whose every answer is a revoked refusal, as the writers answer
/// once the session's grant is revoked.
struct Revoked;

impl Writers for Revoked {
    async fn call(
        &self,
        _verified: &Verified,
        _bearer: &str,
        _mcp_session: &str,
        _space: &Did,
        _request: Request<Body>,
    ) -> Response<Body> {
        rpc(
            StatusCode::FORBIDDEN,
            json!({ "error": "pull: 403: revoked", "code": "CREDENTIAL_REVOKED" }),
        )
    }
}

#[dialog_common::test]
async fn a_revoked_grant_is_recorded_as_a_refusal_not_a_failure() {
    let agent = Ed25519Signer::generate().await.unwrap();
    let space = Ed25519Signer::generate().await.unwrap();
    let events = Events::default();
    let sink = events.sink();
    let key = SessionKey::derive("test");
    let tickets = crate::router::hosted::view::TicketKey::derive("test");
    let bearer = token(&agent, &home()).await;

    // The session's own space: the agent space's writer refuses the read
    // of its grants.
    let endpoint = recording(&key, &tickets, &Revoked, &sink);
    let session = connect(&endpoint, &bearer).await;
    let (_, error) = call_tool(&endpoint, &bearer, &session, "tonk_list_spaces", json!({})).await;
    assert!(error);
    let event = events.last();
    assert_eq!(event.outcome, telemetry::Outcome::Refused);
    assert_eq!(event.kind.as_deref(), Some("credential_revoked"));

    // A granted space: the front still lists the grant, and the space's
    // writer is refused at its pull.
    let writers = Scripted {
        agent: agent.did(),
        access: writable(&agent.did(), &space.did()),
        answer: || {
            rpc(
                StatusCode::FORBIDDEN,
                json!({ "error": "pull: 403: revoked", "code": "CREDENTIAL_REVOKED" }),
            )
        },
        traces: Default::default(),
    };
    let endpoint = recording(&key, &tickets, &writers, &sink);
    let session = connect(&endpoint, &bearer).await;
    let (_, error) = call_tool(
        &endpoint,
        &bearer,
        &session,
        "tonk_evaluate",
        json!({ "space": space.did().to_string(), "document": "x!: {}" }),
    )
    .await;
    assert!(error);
    let event = events.last();
    assert_eq!(event.outcome, telemetry::Outcome::Refused);
    assert_eq!(event.kind.as_deref(), Some("credential_revoked"));
}
