//! A space's socket on the local access service, driven by dialog's own
//! client: a watch begun over the socket is told of each write to its
//! cell, made over HTTP or over a socket, and a frame the socket does not
//! serve is refused.
//!
//! Native only, as every test against the local service is: the service
//! runs natively and these bodies provision it directly. The browser side
//! of the same socket is dialog's web client, run in the browser against
//! dialog's own test service by `dialog-remote-ucan`'s
//! `it_watches_a_cell_over_the_socket` and
//! `it_reads_and_writes_a_cell_over_the_socket`. The worker's twin of
//! this socket is covered by `tests/live.rs`.

#![cfg(feature = "integration-tests")]

use dialog_capability::access::{Authorization as _, AuthorizeError, TimeRange};
use dialog_capability::{Ability, Capability, Effect, ForkInvocation, Provider, Subject};
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_effects::memory::prelude::CellScope;
use dialog_effects::memory::{Edition, MemoryError, Publish, Resolve, Watch};
use dialog_remote_ucan::socket::{PING, PONG, Reply, Request, SUBPROTOCOL};
use dialog_remote_ucan::{UcanAddress, UcanAuthorization, UcanSite};
use dialog_ucan::Scope;
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_ucan_core::{Container, DelegationBuilder, DelegationChain};
use dialog_varsig::Principal as _;
use futures_util::{SinkExt as _, StreamExt as _};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tonk_access_service::helpers::AccessServiceAddress;

/// The invocation `signer` mints for `capability` on its own authority, as
/// dialog's client mints it: saying when it was issued.
async fn issued<Fx>(signer: &Ed25519Signer, capability: &Capability<Fx>) -> UcanAuthorization
where
    Fx: Effect + Clone,
    Capability<Fx>: Ability,
{
    issued_under(signer, None, capability).await
}

/// The invocation `signer` mints for `capability`, proving its authority
/// with `chain` when it is not its own.
async fn issued_under<Fx>(
    signer: &Ed25519Signer,
    chain: Option<DelegationChain>,
    capability: &Capability<Fx>,
) -> UcanAuthorization
where
    Fx: Effect + Clone,
    Capability<Fx>: Ability,
{
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default();
    let minting = dialog_ucan::UcanAuthorization {
        chain,
        signer: Signer::from(signer.clone()),
        scope: Scope::invoke(capability),
        duration: TimeRange {
            not_before: Some(at),
            expiration: Some(at + 60),
        },
        meta: None,
    };
    UcanAuthorization::from(minting.invoke().await.expect("the invocation mints"))
}

/// The service's endpoint, as a client is handed it.
fn endpoint(env: &AccessServiceAddress) -> UcanAddress {
    UcanAddress::new(format!(
        "{}/ucan/",
        env.access_service_url.trim_end_matches('/')
    ))
}

/// The endpoint with its socket, where the local service announces it in
/// its DID document: the endpoint itself, over `ws`.
fn with_socket(env: &AccessServiceAddress) -> UcanAddress {
    let address = endpoint(env);
    let socket = address.endpoint().replacen("http://", "ws://", 1);
    address.with_socket(socket)
}

/// `capability`, invoked by `signer` at `address`.
async fn perform<Fx>(
    address: UcanAddress,
    signer: &Ed25519Signer,
    capability: Capability<Fx>,
) -> Fx::Output
where
    Fx: Effect + Clone,
    Capability<Fx>: Ability,
    UcanSite: Provider<ForkInvocation<UcanSite, Fx>>,
{
    perform_under(address, signer, None, capability).await
}

/// `capability`, invoked by `signer` at `address` under `chain`.
async fn perform_under<Fx>(
    address: UcanAddress,
    signer: &Ed25519Signer,
    chain: Option<DelegationChain>,
    capability: Capability<Fx>,
) -> Fx::Output
where
    Fx: Effect + Clone,
    Capability<Fx>: Ability,
    UcanSite: Provider<ForkInvocation<UcanSite, Fx>>,
{
    let authorization = issued_under(signer, chain, &capability).await;
    UcanSite::default()
        .execute(ForkInvocation::new(capability, address, authorization))
        .await
}

/// A space the service serves, with the signer that owns it.
async fn served(env: &AccessServiceAddress) -> anyhow::Result<(Ed25519Signer, Subject)> {
    let signer = Ed25519Signer::generate().await?;
    env.provision_subject(signer.did().as_str()).await?;
    let subject = Subject::from(signer.did());
    Ok((signer, subject))
}

/// A watch over the socket answers what the cell holds when it begins,
/// then a publish made over HTTP, then one made over the socket.
#[dialog_common::test]
async fn it_tells_a_watch_of_each_write_to_its_cell(
    env: AccessServiceAddress,
) -> anyhow::Result<()> {
    let (signer, subject) = served(&env).await?;
    let cell = CellScope::new(subject, "branch/main", "revision");
    let mut editions = perform::<Watch>(with_socket(&env), &signer, cell.watch()).await?;
    assert_eq!(editions.next().await?, Some(None), "the cell begins empty");

    // The address names no socket, so this goes as a request.
    let version = perform::<Publish>(
        endpoint(&env),
        &signer,
        cell.publish(b"over http".to_vec(), None),
    )
    .await?;
    assert_eq!(
        editions.next().await?,
        Some(Some(Edition {
            content: b"over http".to_vec(),
            version: version.clone(),
        })),
        "a write over HTTP reaches the watch"
    );

    let version = perform::<Publish>(
        with_socket(&env),
        &signer,
        cell.publish(b"over the socket".to_vec(), Some(version)),
    )
    .await?;
    assert_eq!(
        editions.next().await?,
        Some(Some(Edition {
            content: b"over the socket".to_vec(),
            version,
        })),
        "a write over a socket reaches the watch"
    );
    Ok(())
}

/// A watch begun under a grant is told nothing written after the grant
/// is revoked: the next write to its cell ends it, with the revocation as
/// the reason, rather than reaching it.
///
/// The service checks a watch's authority again only after it has stood
/// unchecked a while, so without the revocation forcing that check a
/// holder revoked just after its watch began went on being told of every
/// write for that long. A replica told of a head adopts it, and one that
/// adopts a head it may no longer read cannot then commit on top of it.
#[dialog_common::test]
async fn it_ends_a_watch_whose_grant_was_revoked(env: AccessServiceAddress) -> anyhow::Result<()> {
    let (space, subject) = served(&env).await?;
    let guest = Ed25519Signer::generate().await?;
    let grant = DelegationBuilder::new()
        .issuer(Signer::from(space.clone()))
        .audience(&guest.did())
        .subject(UcanSubject::Specific(space.did()))
        .command(vec![])
        .try_build()
        .await?;
    let chain = DelegationChain::new(grant.clone());
    let cell = CellScope::new(subject, "branch/main", "revision");
    let mut editions =
        perform_under::<Watch>(with_socket(&env), &guest, Some(chain.clone()), cell.watch())
            .await?;
    assert_eq!(editions.next().await?, Some(None), "the cell begins empty");

    // The positive control: the grant's watch is told of a write.
    let version = perform::<Publish>(
        endpoint(&env),
        &space,
        cell.publish(b"before revocation".to_vec(), None),
    )
    .await?;
    assert_eq!(
        editions.next().await?,
        Some(Some(Edition {
            content: b"before revocation".to_vec(),
            version: version.clone(),
        })),
        "the guest's watch is told of a write before revocation"
    );

    let revocation =
        tonk_identity::revocation::mint_root_revocation(space.clone(), &chain, &grant.to_cid())
            .await?;
    let recorded = reqwest::Client::new()
        .post(env.ucan_endpoint())
        .header("Content-Type", "application/cbor")
        .body(revocation)
        .send()
        .await?;
    assert!(
        recorded.status().is_success(),
        "the revocation must be accepted, got {}",
        recorded.status()
    );

    perform::<Publish>(
        endpoint(&env),
        &space,
        cell.publish(b"after revocation".to_vec(), Some(version)),
    )
    .await?;
    let told = editions.next().await;
    assert!(
        matches!(
            told,
            Err(MemoryError::Authorization(AuthorizeError::Revoked { .. }))
        ),
        "a revoked grant's watch must end on the next write, not be told of it, got {told:?}"
    );
    Ok(())
}

/// A space the service does not serve is not watched: the watch is
/// declined as a request for that space would be, and the refusal is the
/// answer to beginning it.
#[dialog_common::test]
async fn it_refuses_a_watch_on_a_space_it_does_not_serve(
    env: AccessServiceAddress,
) -> anyhow::Result<()> {
    let signer = Ed25519Signer::generate().await?;
    let cell = CellScope::new(Subject::from(signer.did()), "branch/main", "revision");
    let refused = perform::<Watch>(with_socket(&env), &signer, cell.watch())
        .await
        .map(|_| ());
    assert!(
        matches!(
            refused,
            Err(MemoryError::Authorization(AuthorizeError::Declined { .. }))
        ),
        "an unserved space's watch is declined, got {refused:?}"
    );
    Ok(())
}

/// A socket serves the space it was opened for: a frame invoking another
/// space's publish, though that space is served and the invocation is its
/// owner's, is declined on it, and the cell is not written.
#[dialog_common::test]
async fn it_refuses_a_frame_for_another_space(env: AccessServiceAddress) -> anyhow::Result<()> {
    let (_, socket_space) = served(&env).await?;
    let (signer, other) = served(&env).await?;
    let cell = CellScope::new(other, "branch/main", "revision");
    let publish = issued(&signer, &cell.publish(b"misdirected".to_vec(), None)).await;

    let socket = with_socket(&env)
        .socket()
        .expect("the endpoint has a socket")
        .to_string();
    let mut request = format!("{socket}?sub={}", socket_space.did()).into_client_request()?;
    request
        .headers_mut()
        .insert("Sec-WebSocket-Protocol", SUBPROTOCOL.parse()?);
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await?;
    let frame = Request::Invoke {
        id: Some(7),
        container: Container::from(publish.invocation().chain()).to_bytes()?,
        payload: Some(b"misdirected".to_vec()),
    };
    socket.send(Message::binary(frame.encode())).await?;
    let answer = loop {
        match socket.next().await {
            Some(Ok(Message::Binary(bytes))) => break Reply::decode(&bytes)?,
            Some(Ok(_)) => continue,
            other => anyhow::bail!("the socket closed without an answer: {other:?}"),
        }
    };
    let Reply::Answer {
        id, status, body, ..
    } = answer
    else {
        anyhow::bail!("expected an answer, got {answer:?}");
    };
    assert_eq!(id, Some(7), "the refusal names the frame it answers");
    assert_eq!(status, 403);
    let reason: AuthorizeError = serde_json::from_slice(&body)?;
    assert!(
        matches!(reason, AuthorizeError::Declined { .. }),
        "the frame is declined, got {reason:?}"
    );

    let held = perform::<Resolve>(endpoint(&env), &signer, cell.resolve()).await?;
    assert_eq!(held, None, "the refused frame wrote nothing");
    Ok(())
}

/// A socket's `ping` is answered with `pong`, as text, without being read
/// as a frame, so dialog's client keeps a quiet connection open. The
/// worker's object answers it through the runtime's auto-response, which
/// no test here runs: Miniflare's `tests/live.rs` exchanges frames only.
#[dialog_common::test]
async fn it_answers_a_ping_with_a_pong(env: AccessServiceAddress) -> anyhow::Result<()> {
    let (_, space) = served(&env).await?;
    let socket = with_socket(&env)
        .socket()
        .expect("the endpoint has a socket")
        .to_string();
    let mut request = format!("{socket}?sub={}", space.did()).into_client_request()?;
    request
        .headers_mut()
        .insert("Sec-WebSocket-Protocol", SUBPROTOCOL.parse()?);
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await?;
    socket.send(Message::text(PING)).await?;
    match socket.next().await {
        Some(Ok(Message::Text(text))) => assert_eq!(text.as_str(), PONG),
        other => anyhow::bail!("expected a pong, got {other:?}"),
    }
    Ok(())
}
