//! Ticket links: an invite whose grant waits in the space rather than
//! riding in the URL.
//!
//! ```text
//! <origin>/space/<space did>#<base58 seed>
//! ```
//!
//! The inviter delegates to a fresh key, as an audience-open invite
//! does, and leaves the delegation chain in the space's memory as the
//! key's ticket (`ticket/{key did}`, see `dialog_effects::ticket`). The
//! link carries only the space and the key's seed. The redeemer derives
//! the key from the seed, signs a `/ucan/claim` as it against the access
//! service at the link's origin, and gets the chain back, which then
//! redeems exactly as an audience-open invite does.
//!
//! The link is short and stable: it is the space's own address with the
//! seed beside it, so it opens the space for whoever already has it, and
//! hands a newcomer what they need to get it. Taking the ticket out of
//! the space hides the grant from the next claim; revoking the
//! delegation is what withdraws it from a redeemer who already claimed.
//!
//! This module only reads and writes the link, and turns a fetched
//! ticket into an [`Invite`]. Fetching it is the caller's, over the
//! transport it already speaks.

use anyhow::{Context, Result};
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_ucan_core::DelegationChain;
use dialog_varsig::{Did, Principal};
use url::Url;

use crate::{EphemeralSeed, Invite, InviteAudience, SEED_LEN, home_address};

/// The path segment a space's address sits under: `/space/{did}`.
const SPACE_PATH: &str = "space";

/// A ticket link: the space it opens, the seed of the key the space
/// keeps a ticket for, and the access service the ticket is claimed
/// from.
#[derive(Debug, Clone)]
pub struct Ticket {
    subject: Did,
    seed: EphemeralSeed,
    remote: Url,
}

impl Ticket {
    /// A ticket for the key `seed` derives, kept by `subject` and
    /// claimed from the access service at `remote`.
    ///
    /// The link is rooted at `remote`'s origin, which is where the space
    /// is served and where the redeemer will claim: `{origin}/ucan/`.
    ///
    /// # Errors
    ///
    /// Returns an error if `remote` has no origin to root a link at.
    pub fn new(subject: Did, seed: EphemeralSeed, remote: &Url) -> Result<Self> {
        Ok(Self {
            subject,
            seed,
            remote: claim_endpoint(remote)?,
        })
    }

    /// The ticket a published space keeps for the public principal (see
    /// [`crate::public`]), claimed from the access service at `remote`.
    ///
    /// # Errors
    ///
    /// Returns an error if `remote` has no origin to claim at.
    pub fn public(subject: Did, remote: &Url) -> Result<Self> {
        Self::new(subject, crate::public::seed(), remote)
    }

    /// Whether this is the public principal's ticket rather than an
    /// invite's: it makes its redeemer a reader, not a member.
    pub fn is_public(&self) -> bool {
        self.seed == crate::public::seed()
    }

    /// The public ticket of the space a bare space address opens,
    /// `/space/{did}` with no seed beside it, or `None` when `url` is not
    /// one.
    ///
    /// # Errors
    ///
    /// Returns an error only when `url` does not parse as a URL.
    pub fn public_for_url(url: &str) -> Result<Option<Self>> {
        let parsed = Url::parse(url).context("space address is not a valid URL")?;
        if parsed.fragment().and_then(decode_seed).is_some() {
            return Ok(None);
        }
        let Some(subject) = space_subject(&parsed) else {
            return Ok(None);
        };
        Self::public(subject, &parsed).map(Some)
    }

    /// The space the ticket opens.
    pub fn subject(&self) -> &Did {
        &self.subject
    }

    /// The access service the ticket is claimed from.
    pub fn remote(&self) -> &Url {
        &self.remote
    }

    /// The key the space keeps the ticket for, which signs the claim.
    ///
    /// # Errors
    ///
    /// Returns an error if the seed does not import as an Ed25519 key.
    pub async fn holder(&self) -> Result<Signer> {
        Ed25519Signer::import(&self.seed)
            .await
            .map(Signer::from)
            .context("failed to import the ticket key from its seed")
    }

    /// The link: `{origin}/space/{subject}#{seed}`, or for the public
    /// ticket the space's bare address, since its seed is everyone's.
    ///
    /// # Errors
    ///
    /// Returns an error if the link does not assemble.
    pub fn to_url(&self) -> Result<String> {
        let mut url = self
            .remote
            .join(&format!("/{SPACE_PATH}/{}", self.subject))
            .context("ticket link did not assemble")?;
        if !self.is_public() {
            url.set_fragment(Some(&bs58::encode(self.seed).into_string()));
        }
        Ok(url.into())
    }

    /// Read a ticket link, or `None` when `url` is not one.
    ///
    /// A ticket link is a space's address, `/space/{did}`, with a
    /// fragment that decodes to a key seed. The same address without one
    /// is just the space, and a fragment that is not a seed is the page's
    /// own; neither is a ticket, and neither is an error. Query
    /// parameters (referral attribution) are ignored.
    ///
    /// # Errors
    ///
    /// Returns an error only when `url` does not parse as a URL.
    pub fn parse_url(url: &str) -> Result<Option<Self>> {
        let parsed = Url::parse(url).context("ticket link is not a valid URL")?;
        let Some(subject) = space_subject(&parsed) else {
            return Ok(None);
        };
        let Some(seed) = parsed.fragment().and_then(decode_seed) else {
            return Ok(None);
        };
        Ok(Some(Self {
            subject,
            seed,
            remote: claim_endpoint(&parsed)?,
        }))
    }

    /// Redeem the fetched `ticket` as the audience-open invite it is.
    ///
    /// # Errors
    ///
    /// Returns an error if the ticket is not a delegation chain, if it
    /// grants a space other than the one the link names, or if its
    /// audience is not the key the link's seed derives.
    pub async fn redeem(self, ticket: &[u8]) -> Result<Invite> {
        let chain = self.accept(ticket).await?;
        // The chain's own signed endpoint wins, as it does on an
        // `access=` invite; the link's origin is where it was claimed.
        let remote = home_address(&chain)?.unwrap_or(self.remote);
        Invite::new(
            chain,
            InviteAudience::Open { seed: self.seed },
            Some(remote),
        )
        .await
    }

    /// Read the fetched `ticket` as the delegation chain it is, checking
    /// that it grants the space the link opens to the key the link's
    /// seed derives.
    ///
    /// # Errors
    ///
    /// Returns an error if the ticket is not a delegation chain, grants
    /// another space, or was kept for another key.
    pub async fn accept(&self, ticket: &[u8]) -> Result<DelegationChain> {
        let chain = DelegationChain::try_from(ticket)
            .context("the space's ticket is not a delegation chain")?;
        anyhow::ensure!(
            chain.subject() == Some(&self.subject),
            "the ticket grants {} but the link opens {}",
            chain
                .subject()
                .map_or_else(|| "any subject".to_owned(), ToString::to_string),
            self.subject,
        );
        let holder = Self::holder_did(&self.seed).await?;
        anyhow::ensure!(
            chain.audience() == &holder,
            "the ticket was kept for {} but the link holds {holder}",
            chain.audience(),
        );
        Ok(chain)
    }

    /// The DID of the key a seed derives: the holder a space keeps a
    /// ticket for.
    ///
    /// # Errors
    ///
    /// Returns an error if the seed does not import as an Ed25519 key.
    pub async fn holder_did(seed: &EphemeralSeed) -> Result<Did> {
        Ok(Ed25519Signer::import(seed)
            .await
            .context("failed to import the ticket key from its seed")?
            .did())
    }
}

/// The access endpoint on `url`'s origin, with any credentials it
/// carried dropped: a link is pasted to whoever is being invited.
fn claim_endpoint(url: &Url) -> Result<Url> {
    let mut origin = url.clone();
    let _ = origin.set_username("");
    let _ = origin.set_password(None);
    origin.set_query(None);
    origin.set_fragment(None);
    origin
        .join("/ucan/")
        .with_context(|| format!("'{url}' has no origin to claim a ticket at"))
}

/// The space a `/space/{did}` address opens.
fn space_subject(url: &Url) -> Option<Did> {
    let mut segments = url.path_segments()?;
    let (Some(SPACE_PATH), Some(subject), None) =
        (segments.next(), segments.next(), segments.next())
    else {
        return None;
    };
    subject.parse().ok()
}

fn decode_seed(fragment: &str) -> Option<EphemeralSeed> {
    let bytes = bs58::decode(fragment).into_vec().ok()?;
    <[u8; SEED_LEN]>::try_from(bytes.as_slice()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_ucan_core::DelegationBuilder;
    use dialog_ucan_core::subject::Subject as UcanSubject;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    const SPACE_SEED: [u8; 32] = [5u8; 32];
    const TICKET_SEED: [u8; 32] = [6u8; 32];

    async fn did(seed: &[u8; 32]) -> Did {
        Ed25519Signer::import(seed).await.unwrap().did()
    }

    /// The chain a space owner leaves as `holder`'s ticket.
    async fn ticket_chain(subject_seed: &[u8; 32], holder: &Did) -> DelegationChain {
        let owner = Ed25519Signer::import(subject_seed).await.unwrap();
        let delegation = DelegationBuilder::new()
            .issuer(Signer::from(owner.clone()))
            .audience(holder)
            .subject(UcanSubject::Specific(owner.did()))
            .command(vec!["use".to_owned()])
            .try_build()
            .await
            .unwrap();
        DelegationChain::new(delegation)
    }

    #[dialog_common::test]
    async fn it_writes_the_space_address_with_the_seed_beside_it() {
        let subject = did(&SPACE_SEED).await;
        let remote = Url::parse("https://user:pass@tonk.example/ucan/").unwrap();
        let ticket = Ticket::new(subject.clone(), TICKET_SEED, &remote).unwrap();

        let url = ticket.to_url().unwrap();
        assert_eq!(
            url,
            format!(
                "https://tonk.example/space/{subject}#{}",
                bs58::encode(TICKET_SEED).into_string()
            )
        );

        let read = Ticket::parse_url(&url).unwrap().expect("a ticket link");
        assert_eq!(read.subject(), &subject);
        assert_eq!(read.remote().as_str(), "https://tonk.example/ucan/");
        assert_eq!(read.holder().await.unwrap().did(), did(&TICKET_SEED).await);
    }

    #[dialog_common::test]
    async fn it_reads_past_referral_parameters() {
        let subject = did(&SPACE_SEED).await;
        let url = format!(
            "https://tonk.example/space/{subject}?tonk_channel=share#{}",
            bs58::encode(TICKET_SEED).into_string()
        );
        let read = Ticket::parse_url(&url).unwrap().expect("a ticket link");
        assert_eq!(read.subject(), &subject);
    }

    #[dialog_common::test]
    async fn it_reads_no_ticket_where_there_is_none() {
        let subject = did(&SPACE_SEED).await;
        let seed = bs58::encode(TICKET_SEED).into_string();
        for url in [
            // The space itself, no seed.
            format!("https://tonk.example/space/{subject}"),
            // A fragment that is the page's own.
            format!("https://tonk.example/space/{subject}#notes"),
            // A path within the space.
            format!("https://tonk.example/space/{subject}/inspector#{seed}"),
            // Not a space's address.
            format!("https://tonk.example/join#{seed}"),
            format!("https://tonk.example/space/not-a-did#{seed}"),
        ] {
            assert!(Ticket::parse_url(&url).unwrap().is_none(), "{url}");
        }
    }

    #[dialog_common::test]
    async fn it_redeems_the_fetched_ticket_as_an_open_invite() {
        let subject = did(&SPACE_SEED).await;
        let holder = did(&TICKET_SEED).await;
        let remote = Url::parse("https://tonk.example/ucan/").unwrap();
        let chain = ticket_chain(&SPACE_SEED, &holder).await;

        let invite = Ticket::new(subject.clone(), TICKET_SEED, &remote)
            .unwrap()
            .redeem(&chain.to_bytes().unwrap())
            .await
            .unwrap();
        assert_eq!(invite.subject(), &subject);
        assert!(matches!(invite.audience, InviteAudience::Open { .. }));
        assert_eq!(invite.remote_url, Some(remote));

        let member = did(&[7u8; 32]).await;
        let claimed = invite.claim(&member).await.unwrap();
        assert_eq!(claimed.chain.audience(), &member);
    }

    #[dialog_common::test]
    async fn it_refuses_a_ticket_for_another_space() {
        let holder = did(&TICKET_SEED).await;
        let elsewhere = ticket_chain(&[8u8; 32], &holder).await;
        let remote = Url::parse("https://tonk.example/ucan/").unwrap();

        let refused = Ticket::new(did(&SPACE_SEED).await, TICKET_SEED, &remote)
            .unwrap()
            .redeem(&elsewhere.to_bytes().unwrap())
            .await;
        assert!(refused.is_err());
    }

    #[dialog_common::test]
    async fn it_refuses_a_ticket_kept_for_another_key() {
        let subject = did(&SPACE_SEED).await;
        let someone_else = did(&[9u8; 32]).await;
        let chain = ticket_chain(&SPACE_SEED, &someone_else).await;
        let remote = Url::parse("https://tonk.example/ucan/").unwrap();

        let refused = Ticket::new(subject, TICKET_SEED, &remote)
            .unwrap()
            .redeem(&chain.to_bytes().unwrap())
            .await;
        assert!(refused.is_err());
    }

    /// A bare space address claims the public principal's ticket; one
    /// with a seed is an invite's ticket link and not a public visit.
    #[dialog_common::test]
    async fn it_reads_a_bare_space_address_as_its_public_ticket() {
        let subject = did(&SPACE_SEED).await;
        let bare = format!("https://tonk.example/space/{subject}?tonk_channel=share");
        let public = Ticket::public_for_url(&bare)
            .unwrap()
            .expect("a space address");
        assert!(public.is_public());
        assert_eq!(
            public.to_url().unwrap(),
            format!("https://tonk.example/space/{subject}")
        );
        assert_eq!(public.subject(), &subject);
        assert_eq!(public.remote().as_str(), "https://tonk.example/ucan/");
        assert_eq!(
            public.holder().await.unwrap().did(),
            crate::public::did().await.unwrap()
        );

        let link = format!(
            "https://tonk.example/space/{subject}#{}",
            bs58::encode(TICKET_SEED).into_string()
        );
        assert!(Ticket::public_for_url(&link).unwrap().is_none());
        assert!(!Ticket::parse_url(&link).unwrap().unwrap().is_public());
        for elsewhere in [
            format!("https://tonk.example/space/{subject}/inspector"),
            "https://tonk.example/join".to_owned(),
        ] {
            assert!(Ticket::public_for_url(&elsewhere).unwrap().is_none());
        }
    }

    #[dialog_common::test]
    async fn it_accepts_the_public_ticket_only_for_the_public_key() {
        let subject = did(&SPACE_SEED).await;
        let remote = Url::parse("https://tonk.example/ucan/").unwrap();
        let ticket = Ticket::public(subject, &remote).unwrap();

        let published = ticket_chain(&SPACE_SEED, &crate::public::did().await.unwrap()).await;
        let accepted = ticket.accept(&published.to_bytes().unwrap()).await.unwrap();
        assert_eq!(accepted.audience(), &crate::public::did().await.unwrap());

        let invited = ticket_chain(&SPACE_SEED, &did(&TICKET_SEED).await).await;
        assert!(ticket.accept(&invited.to_bytes().unwrap()).await.is_err());
    }
}
