//! An agent space a Host Worker under `wrangler dev` can answer MCP over:
//! a native access service, the agent space provisioned there with one note
//! its session wrote through a writer of its kernel, and the agent session
//! token an app would hold. Prints `{ "access", "token", "space" }` as one
//! line, then serves until stopped.
//!
//! cargo run -p tonk-mcp --features native-kernels --example hosted_fixture

use axum::body::Body;
use axum::http::{Method, Request, header};
use dialog_common::helpers::Provisionable as _;
use dialog_credentials::Ed25519Signer;
use dialog_varsig::Principal as _;
use http_body_util::BodyExt as _;
use tonk_access_service::helpers::AccessServiceAddress;
use tonk_mcp::SpaceWriter;
use tonk_mcp::native::NativeKernels;

/// The note the fixture's agent space holds.
const NOTE: &str = "attribute!: &fixture-note-title
  description: Title
  the: fixture.note/title
  as: text
concept!: &fixture-note
  description: A note
  with:
    title: fixture-note-title
fixture-note!:
  this: urn:fixture:note
  title: \"written before the agent connected\"
";

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let service = AccessServiceAddress::start(Default::default())
        .await
        .expect("the access service starts");
    let agent = Ed25519Signer::generate().await.expect("a key");
    service
        .address
        .provision_subject(agent.did().as_ref())
        .await
        .expect("the agent space is provisioned");
    let remote = url::Url::parse(&service.address.ucan_endpoint()).expect("a URL");
    let token = tonk_identity::agent::issue_agent_session(agent.clone(), &remote)
        .await
        .expect("a session")
        .token;

    let verified = tonk_mcp::mcp::verify(&token)
        .await
        .expect("the token verifies");
    let session = verified.writer_session("fixture");
    let writer = SpaceWriter::open(
        NativeKernels::default(),
        &session,
        &agent.did(),
        &remote,
        &[],
    )
    .await
    .expect("the writer opens");
    let written = writer
        .write(
            Request::builder()
                .method(Method::POST)
                .uri("/evaluate?transact=true")
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from(NOTE))
                .expect("a request"),
        )
        .await;
    let status = written.status();
    let body = written
        .into_body()
        .collect()
        .await
        .expect("a body")
        .to_bytes();
    assert!(status.is_success(), "the note: {status}: {body:?}");

    let access = remote.as_str().trim_end_matches("/ucan/").to_owned();
    println!(
        "{}",
        serde_json::json!({ "access": access, "token": token, "space": agent.did().to_string() })
    );
    std::future::pending::<()>().await;
}
