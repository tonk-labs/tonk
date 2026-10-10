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
use dialog_remote_ucan::socket::{Reply, Request, SUBPROTOCOL};
use dialog_remote_ucan::{UcanAddress, UcanAuthorization, UcanSite};
use dialog_repository::SiteAddress;
use dialog_ucan::Scope;
use dialog_ucan_core::Container;
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
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default();
    let minting = dialog_ucan::UcanAuthorization {
        chain: None,
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

/// The endpoint with its socket, found where tonk's client looks for it.
fn with_socket(env: &AccessServiceAddress) -> UcanAddress {
    match tonk_account::peer::with_socket(SiteAddress::Ucan(endpoint(env))) {
        SiteAddress::Ucan(address) => address,
        other => panic!("an access service's address stays one, got {other:?}"),
    }
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
    let authorization = issued(signer, &capability).await;
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

/// A space the service does not serve is not watched: the watch is
/// declined as a request for that space would be. The watch is sent
/// when it begins, and its first answer is the refusal.
#[dialog_common::test]
async fn it_refuses_a_watch_on_a_space_it_does_not_serve(
    env: AccessServiceAddress,
) -> anyhow::Result<()> {
    let signer = Ed25519Signer::generate().await?;
    let cell = CellScope::new(Subject::from(signer.did()), "branch/main", "revision");
    let mut editions = perform::<Watch>(with_socket(&env), &signer, cell.watch()).await?;
    let refused = editions.next().await;
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
    let Reply::Answer { status, body, .. } = answer else {
        anyhow::bail!("expected an answer, got {answer:?}");
    };
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
