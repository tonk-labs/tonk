use dialog_credentials::{Ed25519Signer, Signer};
use dialog_ucan_core::DelegationBuilder;
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_varsig::Principal as _;

use super::*;

fn remote() -> url::Url {
    url::Url::parse("https://tonk.example/ucan/").unwrap()
}

/// An open invite to `space`, as a share link carries one: `space ->
/// link key` for `command` (`[]` is every command), the key's seed beside
/// it, expiring at `expires` when given.
pub(crate) async fn invite(
    space: &Ed25519Signer,
    command: &[&str],
    expires: Option<Timestamp>,
) -> Invite {
    let seed: [u8; 32] = rand::random();
    let key = Ed25519Signer::import(&seed).await.unwrap();
    let mut root = DelegationBuilder::new()
        .issuer(Signer::from(space.clone()))
        .audience(&key.did())
        .subject(UcanSubject::Specific(space.did()))
        .command(
            command
                .iter()
                .map(|segment| (*segment).to_owned())
                .collect(),
        )
        .meta(tonk_invite::home_address_meta(&remote()));
    if let Some(expires) = expires {
        root = root.expiration(expires);
    }
    let chain = DelegationChain::new(root.try_build().await.unwrap());
    Invite::new(chain, InviteAudience::Open { seed }, Some(remote()))
        .await
        .unwrap()
}

fn verbs(chains: &[DelegationChain]) -> Vec<String> {
    let mut verbs: Vec<String> = chains
        .iter()
        .map(|chain| chain.proofs().last().unwrap().command().0[1].clone())
        .collect();
    verbs.sort();
    verbs.dedup();
    verbs
}

#[dialog_common::test]
async fn a_read_join_grants_only_reads() {
    let space = Ed25519Signer::generate().await.unwrap();
    let agent = Ed25519Signer::generate().await.unwrap().did();
    let offer = Offer::of(invite(&space, &[], None).await).unwrap();
    assert!(offer.write, "a member's link allows writing");
    assert_eq!(offer.subject, space.did());
    let chains = offer
        .grant(&agent, Level::Read, Timestamp::now())
        .await
        .unwrap();
    assert!(!chains.is_empty());
    assert_eq!(verbs(&chains), ["get"]);
    for chain in &chains {
        assert_eq!(chain.audience(), &agent);
        assert_eq!(chain.subject(), Some(&space.did()));
    }
}

#[dialog_common::test]
async fn a_write_join_grants_reads_and_writes() {
    let space = Ed25519Signer::generate().await.unwrap();
    let agent = Ed25519Signer::generate().await.unwrap().did();
    let offer = Offer::of(invite(&space, &[], None).await).unwrap();
    let chains = offer
        .grant(&agent, Level::ReadWrite, Timestamp::now())
        .await
        .unwrap();
    assert_eq!(verbs(&chains), ["get", "put"]);
}

#[dialog_common::test]
async fn a_read_only_link_offers_no_writing() {
    let space = Ed25519Signer::generate().await.unwrap();
    let agent = Ed25519Signer::generate().await.unwrap().did();
    let offer = Offer::of(invite(&space, &["use", "get"], None).await).unwrap();
    assert!(!offer.write);
    let refused = offer
        .grant(&agent, Level::ReadWrite, Timestamp::now())
        .await
        .unwrap_err();
    assert!(refused.contains("not change it"), "{refused}");
    let chains = offer
        .grant(&agent, Level::Read, Timestamp::now())
        .await
        .unwrap();
    assert_eq!(verbs(&chains), ["get"]);
}

#[dialog_common::test]
async fn a_grant_ends_no_later_than_its_link() {
    let space = Ed25519Signer::generate().await.unwrap();
    let agent = Ed25519Signer::generate().await.unwrap().did();
    let now = Timestamp::now();
    let soon = Timestamp::try_from(i128::from(now.to_unix() + 3_600)).unwrap();
    let offer = Offer::of(invite(&space, &[], Some(soon)).await).unwrap();
    let chains = offer.grant(&agent, Level::Read, now).await.unwrap();
    for chain in &chains {
        assert_eq!(
            chain.expiration().map(|at| at.to_unix()),
            Some(soon.to_unix())
        );
    }

    let unbounded = Offer::of(invite(&space, &[], None).await).unwrap();
    let chains = unbounded.grant(&agent, Level::Read, now).await.unwrap();
    assert_eq!(
        chains[0].expiration().map(|at| at.to_unix()),
        Some(now.to_unix() + DEFAULT_GRANT_TTL_SECONDS)
    );
}

#[dialog_common::test]
async fn an_expired_link_offers_nothing() {
    let space = Ed25519Signer::generate().await.unwrap();
    let past = Timestamp::try_from(i128::from(Timestamp::now().to_unix() - 60)).unwrap();
    let refused = Offer::of(invite(&space, &[], Some(past)).await).unwrap_err();
    assert!(refused.contains("expired"), "{refused}");
}

#[dialog_common::test]
async fn an_invite_addressed_to_someone_is_refused() {
    let space = Ed25519Signer::generate().await.unwrap();
    let person = Ed25519Signer::generate().await.unwrap();
    let chain = DelegationChain::new(
        DelegationBuilder::new()
            .issuer(Signer::from(space.clone()))
            .audience(&person.did())
            .subject(UcanSubject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap(),
    );
    let scoped = Invite::new(chain, InviteAudience::Scoped, Some(remote()))
        .await
        .unwrap();
    let refused = Offer::of(scoped).unwrap_err();
    assert!(refused.contains("addressed to one person"), "{refused}");
}

#[dialog_common::test]
async fn a_grant_body_opens_only_for_its_agent_and_space() {
    let space = Ed25519Signer::generate().await.unwrap();
    let agent = Ed25519Signer::generate().await.unwrap().did();
    let other = Ed25519Signer::generate().await.unwrap().did();
    let offer = Offer::of(invite(&space, &[], None).await).unwrap();
    let chains = offer
        .grant(&agent, Level::Read, Timestamp::now())
        .await
        .unwrap();
    let body = GrantBody::new(&offer, &chains).unwrap();
    let body: GrantBody = serde_json::from_str(&serde_json::to_string(&body).unwrap()).unwrap();

    let (opened, entry) = body.open(&agent).unwrap();
    assert_eq!(opened.len(), chains.len());
    assert_eq!(entry.space, space.did());
    assert_eq!(entry.address.as_deref(), Some(remote().as_str()));

    assert!(body.open(&other).is_err(), "addressed to another agent");
    let mut elsewhere = body;
    elsewhere.space = other.to_string();
    assert!(elsewhere.open(&agent).is_err(), "grants another space");
}

#[dialog_common::test]
async fn a_link_that_is_not_one_is_refused_before_any_fetch() {
    for link in [
        "",
        "not a link",
        "ftp://tonk.example/space/x",
        "http://tonk.example/space/x",
    ] {
        let refused = read_link(link, None).await.unwrap_err();
        assert!(!refused.contains(link) || link.is_empty(), "{refused}");
    }
}

/// A `/@/` shortcut on a host that is not Tonk's is refused before it is
/// fetched, so a pasted link cannot make this service request any URL.
/// `example.invalid` cannot resolve: if it were fetched, the refusal would
/// be the fetch failure's, not this one.
#[dialog_common::test]
async fn a_shortcut_on_another_host_is_refused_before_any_fetch() {
    for (link, origin) in [
        ("https://example.invalid/@/abc#seed", None),
        (
            "https://example.invalid/@/abc#seed",
            Some("https://tonk.network"),
        ),
        ("https://tonk.network.example.invalid/@/abc#seed", None),
        ("https://tonk.network:8443/@/abc#seed", None),
        ("http://localhost:9/@/abc#seed", None),
        (
            "http://localhost:9/@/abc#seed",
            Some("http://localhost:8787"),
        ),
    ] {
        let refused = read_link(link, origin).await.unwrap_err();
        assert_eq!(refused, "That is not a Tonk invite link.", "{link}");
    }
}

#[dialog_common::test]
fn shortcuts_expand_only_on_tonk_origins_and_the_served_one() {
    let url = |link: &str| url::Url::parse(link).unwrap();
    assert!(shortcut_origin(&url("https://tonk.network/@/a#s"), None));
    assert!(shortcut_origin(&url("https://tonk.foundation/@/a#s"), None));
    assert!(shortcut_origin(
        &url("http://localhost:8787/@/a#s"),
        Some("http://localhost:8787/")
    ));
    assert!(shortcut_origin(
        &url("https://staging.tonk.example/@/a#s"),
        Some("https://staging.tonk.example")
    ));
    assert!(!shortcut_origin(&url("http://tonk.network/@/a#s"), None));
    assert!(!shortcut_origin(&url("https://evil.example/@/a#s"), None));
    assert!(!shortcut_origin(
        &url("https://evil.example/@/a#s"),
        Some("https://tonk.network")
    ));
}

#[dialog_common::test]
fn the_join_tool_is_the_models_and_the_accept_tool_the_cards() {
    let tools = tools();
    let join = tools.iter().find(|tool| tool["name"] == JOIN).unwrap();
    assert_eq!(join["_meta"]["ui"]["resourceUri"], CARD_URI);
    assert_eq!(join["annotations"]["readOnlyHint"], true);
    let accept = tools.iter().find(|tool| tool["name"] == ACCEPT).unwrap();
    assert_eq!(accept["_meta"]["ui"]["visibility"], json!(["app"]));
    assert_eq!(accept["_meta"]["openai/visibility"], "private");
    assert_eq!(accept["annotations"]["readOnlyHint"], false);
    assert_eq!(accept["annotations"]["destructiveHint"], false);
    let card = read_resource(CARD_URI).unwrap();
    let text = card["contents"][0]["text"].as_str().unwrap();
    assert!(text.contains(ACCEPT), "the card calls the accept tool");
    assert!(read_resource("ui://tonk/other").is_none());
}
