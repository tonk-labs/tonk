//! Typed invitation routing before `tonk join` mutates local state.

mod common;

use anyhow::Result;
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_ucan_core::time::{Duration, SystemTime, Timestamp};
use dialog_ucan_core::{DelegationBuilder, DelegationChain};
use dialog_varsig::Principal as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::RwLock;
use tonk_cli::join::PreparedInvitation;
use tonk_invite::connection::{AgentInvite, candidate_build_scopes};
use tonk_schema::prelude::DidExt as _;

async fn agent_link(base: &str, expired: bool) -> Result<String> {
    let owner = Signer::from(Ed25519Signer::import(&[71; 32]).await?);
    let seed = [72; 32];
    let recipient = Ed25519Signer::import(&seed).await?;
    let remote = url::Url::parse("https://access.example.test/ucan/")?;
    let scopes = candidate_build_scopes(&owner.did());
    let now = Timestamp::now();
    let expiration = if expired {
        Timestamp::try_from((now.to_unix() - 60) as i128)?
    } else {
        Timestamp::new(SystemTime::now() + Duration::from_secs(3600))?
    };
    let validation_time = if expired {
        Timestamp::try_from((now.to_unix() - 120) as i128)?
    } else {
        now
    };
    let mut chains = Vec::new();
    for scope in &scopes {
        let grant = DelegationBuilder::new()
            .issuer(owner.clone())
            .audience(&recipient.did())
            .subject(scope.subject.clone())
            .command(scope.command.0.clone())
            .policy(scope.policy())
            .expiration(expiration)
            .meta(tonk_invite::home_address_meta(&remote))
            .try_build()
            .await?;
        chains.push(DelegationChain::new(grant));
    }
    Ok(
        AgentInvite::new(seed, chains, &scopes, &remote, validation_time)
            .await?
            .to_url(base)?,
    )
}

fn secret_free(error: &anyhow::Error, secrets: &[&str]) {
    let message = format!("{error:#}");
    for secret in secrets {
        assert!(
            !message.contains(secret),
            "error leaked a bearer secret: {message}"
        );
    }
}

#[dialog_common::test]
async fn it_routes_and_validates_full_links_without_fallback() -> Result<()> {
    let issuer = common::TestSite::new().await?;
    let ordinary = tonk_cli::invite::mint(
        &issuer.site,
        Some("https://carrier.example.test/join"),
        None,
    )
    .await?;
    match tonk_cli::join::prepare(&ordinary.url).await? {
        PreparedInvitation::Ordinary(prepared) => {
            assert_eq!(prepared.invitation().subject.0, ordinary.subject.this());
        }
        PreparedInvitation::Agent(_) => panic!("ordinary invitation reached the agent parser"),
    }

    let agent = agent_link("https://carrier.example.test/join", false).await?;
    match tonk_cli::join::prepare(&agent).await? {
        PreparedInvitation::Agent(prepared) => {
            assert_eq!(
                prepared.hint().remote.as_str(),
                "https://access.example.test/ucan/"
            );
        }
        PreparedInvitation::Ordinary(_) => panic!("agent invitation reached the ordinary parser"),
    }

    let mut mixed = url::Url::parse(&agent)?;
    mixed
        .query_pairs_mut()
        .append_pair("access", "ordinary-proof");
    let error = tonk_cli::join::prepare(mixed.as_str()).await.unwrap_err();
    assert!(format!("{error:#}").contains("ambiguous_invitation"));
    secret_free(&error, &["ordinary-proof", "tonk-agent-v"]);

    let unsupported = agent.replace("tonk-agent-v2=", "tonk-agent-v99=");
    let error = tonk_cli::join::prepare(&unsupported).await.unwrap_err();
    assert!(format!("{error:#}").contains("connection_unsupported_version"));
    secret_free(&error, &["tonk-agent-v99"]);

    let malformed = "https://carrier.example.test/join?access=not-base58#never-print-secret";
    let error = tonk_cli::join::prepare(malformed).await.unwrap_err();
    assert!(format!("{error:#}").contains("invalid invite"));
    secret_free(&error, &["not-base58", "never-print-secret"]);

    let expired = agent_link("https://carrier.example.test/join", true).await?;
    let error = tonk_cli::join::prepare(&expired).await.unwrap_err();
    assert!(format!("{error:#}").contains("Expired at"), "{error:#}");
    secret_free(&error, &["tonk-agent-v2"]);
    Ok(())
}

#[dialog_common::test]
async fn shortcuts_resolve_once_before_routing_both_formats() -> Result<()> {
    use axum::Router;
    use axum::http::{HeaderValue, StatusCode, header};
    use axum::response::IntoResponse;

    let location = Arc::new(RwLock::new(String::new()));
    let requests = Arc::new(AtomicUsize::new(0));
    let app = Router::new().fallback({
        let location = location.clone();
        let requests = requests.clone();
        move || {
            let location = location.clone();
            let requests = requests.clone();
            async move {
                requests.fetch_add(1, Ordering::SeqCst);
                let location = location.read().await.clone();
                (
                    StatusCode::TEMPORARY_REDIRECT,
                    [(header::LOCATION, HeaderValue::from_str(&location).unwrap())],
                )
                    .into_response()
            }
        }
    });
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let base = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let server = tokio::spawn(async move { axum::serve(listener, app).await });

    let issuer = common::TestSite::new().await?;
    let ordinary =
        tonk_cli::invite::mint(&issuer.site, Some(&format!("{base}/join")), None).await?;
    let ordinary_url = url::Url::parse(&ordinary.url)?;
    let ordinary_fragment = ordinary_url.fragment().unwrap();
    let mut ordinary_location = ordinary_url.clone();
    ordinary_location.set_fragment(None);
    *location.write().await = ordinary_location.to_string();
    let short = format!("{base}/@/{}#{ordinary_fragment}", "a".repeat(64));
    assert!(matches!(
        tonk_cli::join::prepare(&short).await?,
        PreparedInvitation::Ordinary(_)
    ));
    assert_eq!(requests.load(Ordering::SeqCst), 1);

    let agent = agent_link(&format!("{base}/join"), false).await?;
    let agent_url = url::Url::parse(&agent)?;
    let agent_fragment = agent_url.fragment().unwrap();
    let mut agent_location = agent_url.clone();
    agent_location.set_fragment(None);
    *location.write().await = agent_location.to_string();
    let short = format!("{base}/@/{}#{agent_fragment}", "b".repeat(64));
    assert!(matches!(
        tonk_cli::join::prepare(&short).await?,
        PreparedInvitation::Agent(_)
    ));
    assert_eq!(requests.load(Ordering::SeqCst), 2);

    server.abort();
    Ok(())
}
