//! `tonk:sponsor-agent`: a person sponsors an agent's key.
//!
//! An agent's host (a hosted MCP server) sends the person to
//! `/settings/agent?agent=&callback=&name=`. On approval this device extends
//! its `account -> device` grant to the agent with exactly `/provider/add` on
//! the account (see [`tonk_invite::sponsorship`]) and sends the page to the
//! host's callback. The fragment carries the chain, the account and the
//! account's encryption key, so the agent can make spaces the person owns.
//!
//! No passkey is asked for: the grant lets the agent provision spaces billed
//! to the account and nothing else, and this device already acts for the
//! account.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use dialog_ucan_core::time::Timestamp;
use dialog_varsig::Did;
use tonk_invite::sponsorship::AgentSponsorship;
use tonk_schema::{ceremony, ceremony_state};

use crate::worker::TonkState;

/// A decoded sponsorship request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sponsorship {
    /// The agent to sponsor.
    pub agent: Did,
    /// The host's callback URL, decoded from base58.
    pub callback: String,
    /// What the host calls the agent.
    pub name: String,
}

/// What the agent's host receives, base64 JSON in the `sponsor` fragment
/// field.
#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SponsorDelivery {
    /// The sponsoring account DID.
    pub account: String,
    /// The account's X25519 recipient (`did:key:z6LS…`): what the agent
    /// seals a space seed to so the account owns the space.
    pub recipient: String,
    /// Hex of the `account -> device -> agent` chain for `/provider/add`.
    pub sponsorship_hex: String,
}

/// The progress entity for one request, so another tab or request cannot
/// supply its status. The settings element derives the same entity.
pub(crate) fn sponsorship_entity(request: &Sponsorship) -> String {
    let key = serde_json::to_vec(&[request.agent.to_string(), request.callback.clone()])
        .expect("strings serialize as JSON");
    format!("urn:tonk:sponsorship:{}", bs58::encode(key).into_string())
}

/// Issue the sponsorship and build the callback URL that delivers it.
pub(crate) async fn sponsor(
    tonk: &TonkState,
    request: &Sponsorship,
    now: Timestamp,
) -> Result<String, String> {
    tonk_worker_api::callback::delivery_url(&request.callback, &[])?;
    let root = super::identity::local_root(tonk)
        .await
        .map_err(|_| "sign in to Tonk to sponsor an agent".to_string())?;
    let recipient = match root.encryption_key.clone() {
        Some(recipient) => Some(recipient),
        None => super::account_state::published_sealed_inbox(tonk, &root.root_did)
            .await
            .map_err(|error| error.to_string())?,
    }
    .ok_or_else(|| {
        "this account has no published encryption key yet; open Tonk settings and try again"
            .to_string()
    })?;
    let device = tonk.profile.credential().signer().clone();
    let sponsorship =
        AgentSponsorship::issue_from_device(root.delegation, &device, &request.agent, now)
            .await
            .map_err(|error| format!("the sponsorship was not issued: {error}"))?;
    let delivery = SponsorDelivery {
        account: root.root_did.to_string(),
        recipient: recipient.to_string(),
        sponsorship_hex: hex::encode(
            sponsorship
                .to_bytes()
                .map_err(|error| format!("the sponsorship did not encode: {error}"))?,
        ),
    };
    let payload = STANDARD.encode(serde_json::to_vec(&delivery).expect("delivery serializes"));
    tonk_worker_api::callback::delivery_url(&request.callback, &[("sponsor", &payload)])
}

/// Decode the command, refusing a callback that is not base58 UTF-8.
pub(crate) struct SponsorAgentRequest(Sponsorship);
impl crate::reactor::Decode for SponsorAgentRequest {
    fn trigger_attributes() -> Vec<String> {
        <tonk_schema::command::SponsorAgent as crate::reactor::Decode>::trigger_attributes()
    }
    fn decode(
        entity: dialog_artifacts::Entity,
        facts: &crate::reactor::EntityFacts,
    ) -> Option<Self> {
        let command = tonk_schema::command::SponsorAgent::decode(entity, facts)?;
        let callback = bs58::decode(&command.callback.0).into_vec().ok()?;
        Some(Self(Sponsorship {
            agent: command.agent.0.to_string().parse().ok()?,
            callback: String::from_utf8(callback).ok()?,
            name: command.name.0,
        }))
    }
}
impl dialog_capability::Command for SponsorAgentRequest {
    type Input = Self;
    type Output = ();
}

/// Run `tonk:sponsor-agent`: issue the sponsorship and send the page to the
/// host. A refusal is reported on the request's status and not sent.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<SponsorAgentRequest> for super::CommandEnv {
    async fn execute(&self, command: SponsorAgentRequest) {
        let request = command.0;
        let entity = sponsorship_entity(&request);
        let tonk = self.state().read().await;
        match sponsor(&tonk, &request, Timestamp::now()).await {
            Ok(target) => {
                // The target carries the grant: report done without it.
                super::ceremony::report_to(
                    &tonk,
                    &entity,
                    ceremony::SPONSOR_AGENT,
                    ceremony_state::DONE,
                    "",
                )
                .await;
                super::navigate::notify_navigate(self.client(), &target);
            }
            Err(error) => {
                super::ceremony::report_to(
                    &tonk,
                    &entity,
                    ceremony::SPONSOR_AGENT,
                    ceremony_state::REFUSED,
                    &error,
                )
                .await;
            }
        }
    }
}

#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
mod tests {
    use super::*;
    use dialog_credentials::Ed25519Signer;
    use dialog_varsig::Principal as _;

    async fn agent() -> Did {
        Ed25519Signer::generate().await.unwrap().did()
    }

    /// Sign the device in the way a creation or unlock ceremony does: the
    /// `root -> device` grant, plus the account's encryption key published
    /// on profile main. Returns the account DID.
    async fn sign_in(tonk: &TonkState) -> Did {
        let secret = [7u8; 32];
        let root = Ed25519Signer::import(&secret).await.unwrap();
        let account = root.did();
        let grant = tonk_identity::delegation::mint_device_delegation(root, &tonk.profile.did())
            .await
            .unwrap();
        let recipient =
            tonk_identity::envelope::AccountSecret::from_bytes(zeroize::Zeroizing::new(secret))
                .secret()
                .did();
        super::super::identity::persist_root(
            tonk,
            tonk_worker_api::SaveRootRequest {
                credential_id: "test-credential".to_string(),
                delegation_hex: hex::encode(grant.to_bytes().unwrap()),
                passkey: None,
                encryption_key: Some(recipient.to_string()),
            },
        )
        .await
        .unwrap();
        account
    }

    fn delivered(target: &str) -> SponsorDelivery {
        let url = url::Url::parse(target).unwrap();
        let payload = url::form_urlencoded::parse(url.fragment().unwrap().as_bytes())
            .find(|(key, _)| key == "sponsor")
            .unwrap()
            .1
            .into_owned();
        serde_json::from_slice(&STANDARD.decode(payload).unwrap()).unwrap()
    }

    fn request(agent: Did, callback: &str) -> Sponsorship {
        Sponsorship {
            agent,
            callback: callback.into(),
            name: "ChatGPT".into(),
        }
    }

    #[dialog_common::test]
    async fn it_sponsors_an_agent_for_the_signed_in_account() {
        let state = super::super::command::tests::native::test_state().await;
        let tonk = state.read().await;
        let account = sign_in(&tonk).await;
        let agent = agent().await;
        let now = Timestamp::now();
        let target = sponsor(
            &tonk,
            &request(
                agent.clone(),
                "https://mcp.example/oauth/callback?request=abc",
            ),
            now,
        )
        .await
        .unwrap();
        assert!(target.starts_with("https://mcp.example/oauth/callback?request=abc#"));

        let delivery = delivered(&target);
        assert_eq!(delivery.account, account.to_string());
        assert!(delivery.recipient.starts_with("did:key:z6LS"));
        let verified =
            AgentSponsorship::from_bytes(&hex::decode(&delivery.sponsorship_hex).unwrap())
                .unwrap()
                .verify(&agent, now)
                .await
                .unwrap();
        assert_eq!(verified.sponsor, account);
    }

    #[dialog_common::test]
    async fn it_refuses_when_the_device_is_not_signed_in() {
        let state = super::super::command::tests::native::test_state().await;
        let tonk = state.read().await;
        let error = sponsor(
            &tonk,
            &request(agent().await, "https://mcp.example/oauth/callback"),
            Timestamp::now(),
        )
        .await
        .unwrap_err();
        assert!(error.contains("sign in"), "{error}");
    }

    #[dialog_common::test]
    async fn it_refuses_an_undeliverable_callback_before_signing() {
        let state = super::super::command::tests::native::test_state().await;
        let tonk = state.read().await;
        sign_in(&tonk).await;
        for callback in [
            "http://mcp.example/callback",
            "https://user:pass@mcp.example/callback",
            "javascript:alert(1)",
        ] {
            assert!(
                sponsor(&tonk, &request(agent().await, callback), Timestamp::now())
                    .await
                    .is_err(),
                "accepted {callback}"
            );
        }
    }

    /// A space an agent made for this account: the claim link's sealed key
    /// becomes custody rows the account's next passkey ceremony adopts.
    #[dialog_common::test]
    async fn it_adopts_the_sealed_key_of_a_space_an_agent_made() {
        use dialog_query::{Output as _, Query, Term};
        use dialog_ucan_core::{DelegationBuilder, DelegationChain, subject::Subject};
        use tonk_schema::prelude::DidExt as _;

        let state = super::super::command::tests::native::test_state().await;
        let tonk = state.read().await;
        let account = sign_in(&tonk).await;
        let secret =
            tonk_identity::envelope::AccountSecret::from_bytes(zeroize::Zeroizing::new([7u8; 32]));

        let seed = zeroize::Zeroizing::new([3u8; 32]);
        let space = Ed25519Signer::import(&*seed).await.unwrap();
        let founder = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(space.clone()))
            .audience(&account)
            .subject(Subject::Specific(space.did()))
            .command(Vec::<String>::new())
            .try_build()
            .await
            .unwrap();
        let invite = tonk_invite::Invite::new(
            DelegationChain::new(founder),
            tonk_invite::InviteAudience::Scoped,
            Some("https://hub.test/ucan/".parse().unwrap()),
        )
        .await
        .unwrap();
        let sealed = secret
            .secret()
            .recipient()
            .secret()
            .conceal(&seed, &space.did())
            .unwrap()
            .encode();
        let url = format!(
            "{}#seed={}",
            invite.to_url("https://hub.test/join").unwrap(),
            bs58::encode(&sealed).into_string()
        );

        let (link, found) = super::super::join::split_sealed_seed(&url);
        assert_eq!(found.as_deref(), Some(sealed.as_slice()));
        assert!(!link.contains('#'));
        super::super::join::adopt(&tonk, &link, found.unwrap())
            .await
            .unwrap();
        // An open invite's fragment is a bare seed and stays on the link.
        let open = "https://hub.test/join?access=abc#3mJr7AoUXx2Wqd";
        assert_eq!(
            super::super::join::split_sealed_seed(open),
            (open.to_owned(), None)
        );

        let branch = tonk
            .reactor
            .profile_repository()
            .branch(&tonk.active_branch)
            .acquire(&tonk.operator)
            .await
            .unwrap();
        let principals: Vec<tonk_schema::SecretPrincipal> = branch
            .handle()
            .query()
            .select(Query::<tonk_schema::SecretPrincipal> {
                this: Term::from(space.did().this()),
                kind: Term::var("kind"),
                seed: Term::var("seed"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap();
        assert_eq!(principals.len(), 1, "the sealed key is recorded once");
        let messages: Vec<tonk_schema::SecretMessage> = branch
            .handle()
            .query()
            .select(Query::<tonk_schema::SecretMessage> {
                this: Term::from(principals[0].seed.0.clone()),
                to: Term::var("to"),
                message: Term::var("message"),
                from: Term::var("from"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap();
        let opened = secret
            .secret()
            .reveal(
                &tonk_identity::sealed::Sealed::decode(&messages[0].message.0).unwrap(),
                &space.did(),
            )
            .unwrap();
        assert_eq!(*opened, *seed);

        // A link for another account is not adopted.
        let stranger = Ed25519Signer::generate().await.unwrap().did();
        let other = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(space.clone()))
            .audience(&stranger)
            .subject(Subject::Specific(space.did()))
            .command(Vec::<String>::new())
            .try_build()
            .await
            .unwrap();
        let other = tonk_invite::Invite::new(
            DelegationChain::new(other),
            tonk_invite::InviteAudience::Scoped,
            None,
        )
        .await
        .unwrap()
        .to_url("https://hub.test/join")
        .unwrap();
        assert!(
            super::super::join::adopt(&tonk, &other, sealed)
                .await
                .is_err()
        );
    }
}
