use dialog_credentials::{Ed25519Signer, Signer};
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_ucan_core::{DelegationBuilder, DelegationChain};
use dialog_varsig::Principal as _;
use tonk_invite::InviteAudience;

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

#[dialog_common::test]
async fn a_members_link_offers_reading_and_writing() {
    let space = Ed25519Signer::generate().await.unwrap();
    let offer = Offer::of(invite(&space, &[], None).await).unwrap();
    assert!(offer.write, "a member's link allows writing");
    assert_eq!(offer.subject, space.did());
    assert_eq!(offer.remote, remote());
}

#[dialog_common::test]
async fn a_read_only_link_offers_no_writing() {
    let space = Ed25519Signer::generate().await.unwrap();
    let offer = Offer::of(invite(&space, &["use", "get"], None).await).unwrap();
    assert!(!offer.write);
}

#[dialog_common::test]
async fn an_expired_link_offers_nothing() {
    let space = Ed25519Signer::generate().await.unwrap();
    let past = Timestamp::try_from(i128::from(Timestamp::now().to_unix() - 60)).unwrap();
    let refused = Offer::of(invite(&space, &[], Some(past)).await).unwrap_err();
    assert!(refused.contains("expired"), "{refused}");
}

/// The person claims the invite in Tonk, so one addressed to them is
/// offered too; Tonk refuses it there if it is someone else's.
#[dialog_common::test]
async fn an_invite_addressed_to_someone_is_offered() {
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
    let offer = Offer::of(scoped).unwrap();
    assert_eq!(offer.subject, space.did());
}

#[dialog_common::test]
async fn a_link_that_is_not_one_is_refused_before_any_fetch() {
    for link in [
        "",
        "not a link",
        "ftp://tonk.example/space/x",
        "http://tonk.example/space/x",
    ] {
        let refused = read_link(link).await.unwrap_err();
        assert!(!refused.contains(link) || link.is_empty(), "{refused}");
    }
}

/// The join page carries the agent and the link in its fragment, which
/// the browser never sends to a server.
#[dialog_common::test]
async fn the_join_page_keeps_the_link_in_its_fragment() {
    let agent = Ed25519Signer::generate().await.unwrap().did();
    let link = "https://tonk.example/space/did:key:z6Mk#access=abc&x=1";
    let page = join_page("https://tonk.example/", &agent, link);
    let page = url::Url::parse(&page).unwrap();
    assert_eq!(page.path(), JOIN_PAGE);
    assert_eq!(page.query(), None);
    let fields: std::collections::HashMap<_, _> =
        url::form_urlencoded::parse(page.fragment().unwrap().as_bytes())
            .into_owned()
            .collect();
    assert_eq!(fields["agent"], agent.repo_key());
    assert_eq!(fields["link"], link);
}

#[dialog_common::test]
fn the_join_tool_is_the_models_and_the_status_tool_the_cards() {
    let tools = tools();
    let join = tools.iter().find(|tool| tool["name"] == JOIN).unwrap();
    assert_eq!(join["_meta"]["ui"]["resourceUri"], CARD_URI);
    assert_eq!(join["annotations"]["readOnlyHint"], true);
    let status = tools.iter().find(|tool| tool["name"] == STATUS).unwrap();
    assert_eq!(status["_meta"]["ui"]["visibility"], json!(["app"]));
    assert_eq!(status["_meta"]["openai/visibility"], "private");
    assert!(
        tools.iter().all(|tool| tool["name"] != "tonk_join_accept"),
        "no tool joins with the link's key"
    );
    let card = read_resource(CARD_URI).unwrap();
    let text = card["contents"][0]["text"].as_str().unwrap();
    assert!(text.contains(STATUS), "the card asks with the status tool");
    assert!(text.contains("ui/open-link"), "the card opens Tonk");
    assert!(read_resource("ui://tonk/other").is_none());
}
