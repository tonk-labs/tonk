//! An agent's sessions: the `agent space → session` powerline and the
//! token that carries it.
//!
//! An agent is a space, provisioned by an account and sealed under it.
//! Each authorization of an LLM host mints a fresh session key, and the
//! agent space delegates to it with a powerline: subject-open,
//! command-open and unexpiring, the same shape as the `root → device`
//! grant. A session is ended by revoking that powerline, never by
//! letting it lapse: what a leaked session can reach beyond the agent's
//! own space is bounded by its invites, which do expire, and renewing
//! the powerline would need the account, which holds the agent's key.
//!
//! # The token
//!
//! A host holds one opaque string per session. It is a UCAN container,
//! in the container spec's base64url serialization, of two delegations:
//!
//! - the powerline, `agent space → session`, carrying the agent's sync
//!   endpoint in its signed `meta` ([`tonk_invite::HOME_ADDRESS`]);
//! - on top of it, a delegation the session issues to itself, whose
//!   `meta` says what the token is ([`TOKEN_TYPE`], [`TOKEN_VERSION`])
//!   and carries the session key's seed ([`TOKEN_SEED`]).
//!
//! The outer delegation authorizes nothing: it is issued by the session
//! to itself, about itself. It exists so the token is one self-describing
//! container rather than a chain plus a key in some other envelope.
//! Whoever reads the token derives the key from the seed and checks that
//! its DID is the outer delegation's issuer and audience, and the
//! powerline's audience.
//!
//! Only the powerline is retained (the agent space's session row) and
//! only the powerline is sent as a proof with an invocation, so the seed
//! never reaches storage or a verifier.

use std::collections::BTreeMap;

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use dialog_credentials::{DidKeyResolver, Ed25519Signer, Signer};
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_ucan_core::{Container, DelegationBuilder, DelegationChain, Tag};
use dialog_varsig::Did;
use dialog_varsig::principal::Principal as _;
use ipld_core::ipld::Ipld;
use tonk_invite::{home_address, home_address_meta};
use url::Url;
use zeroize::Zeroizing;

/// `meta` key naming what a token is.
pub const TOKEN_TYPE: &str = "token.type";

/// `meta` key naming the token format's version.
pub const TOKEN_VERSION: &str = "token.version";

/// `meta` key carrying the session key's 32-byte Ed25519 seed.
pub const TOKEN_SEED: &str = "token.seed";

/// `meta` key carrying the grants the agent was given with this session:
/// a list of encoded delegation chains, each delegating some space to
/// the agent space. Absent when there are none.
pub const TOKEN_GRANTS: &str = "token.grants";

/// The [`TOKEN_TYPE`] of an agent session token.
pub const AGENT_SESSION_TOKEN: &str = "agent-session";

/// The [`TOKEN_VERSION`] this module writes and the only one it reads.
pub const AGENT_SESSION_TOKEN_VERSION: u64 = 1;

/// Mint the `agent space → session` powerline: subject-open,
/// command-open, no expiry, naming the agent's sync endpoint `home` in
/// its signed `meta`.
///
/// The shape of [`mint_addressed_device_delegation`]: a session acts as
/// the agent for anything, until the powerline is revoked.
///
/// [`mint_addressed_device_delegation`]: crate::delegation::mint_addressed_device_delegation
pub async fn mint_agent_session(
    agent_space: impl Into<Signer>,
    session: &Did,
    home: &Url,
) -> Result<DelegationChain> {
    let delegation = DelegationBuilder::new()
        .issuer(agent_space.into())
        .audience(session)
        .subject(UcanSubject::Any)
        .command(vec![])
        .meta(home_address_meta(home))
        .try_build()
        .await
        .map_err(|e| anyhow!("failed to mint the agent session powerline: {e}"))?;
    Ok(DelegationChain::new(delegation))
}

/// A freshly minted session: the token for the host, and the powerline
/// to retain in the agent space.
pub struct IssuedAgentSession {
    /// The token, ready to hand to the host. Holds the session's seed.
    pub token: String,
    /// The powerline inside the token: what the agent space retains as
    /// the session row, and what revoking the session withdraws.
    pub powerline: DelegationChain,
}

/// Mint a session for the agent `agent_space` on a fresh key: the
/// powerline from [`mint_agent_session`], wrapped as a token by
/// [`encode_agent_session_token`].
pub async fn issue_agent_session(
    agent_space: impl Into<Signer>,
    home: &Url,
) -> Result<IssuedAgentSession> {
    issue_agent_session_with_grants(agent_space, home, &[]).await
}

/// [`issue_agent_session`], with `grants` carried in the token: chains
/// delegating other spaces to the agent space, so the host can act in
/// them without looking them up.
pub async fn issue_agent_session_with_grants(
    agent_space: impl Into<Signer>,
    home: &Url,
    grants: &[DelegationChain],
) -> Result<IssuedAgentSession> {
    let agent_space = agent_space.into();
    let agent = agent_space.did();
    for grant in grants {
        ensure!(
            grant.audience() == &agent,
            "a grant delegates to {}, not to the agent {agent}",
            grant.audience()
        );
    }
    let mut seed = Zeroizing::new([0u8; 32]);
    getrandom::fill(seed.as_mut())
        .map_err(|error| anyhow!("no entropy for a session key: {error}"))?;
    let session = Ed25519Signer::import(&*seed)
        .await
        .map_err(|error| anyhow!("failed to derive the session key: {error:?}"))?;
    let powerline = mint_agent_session(agent_space, &session.did(), home).await?;
    let token = encode_agent_session_token_with_grants(&seed, &powerline, grants).await?;
    Ok(IssuedAgentSession { token, powerline })
}

/// Wrap `powerline` and the seed of the key it delegates to as a token.
///
/// Refuses a powerline whose audience is not the key `seed` derives:
/// such a token would decode to a key that cannot use its own proof.
pub async fn encode_agent_session_token(
    seed: &Zeroizing<[u8; 32]>,
    powerline: &DelegationChain,
) -> Result<String> {
    encode_agent_session_token_with_grants(seed, powerline, &[]).await
}

/// [`encode_agent_session_token`], carrying `grants` under [`TOKEN_GRANTS`].
///
/// Refuses a grant that does not delegate to the powerline's issuer: the
/// session could never use it.
pub async fn encode_agent_session_token_with_grants(
    seed: &Zeroizing<[u8; 32]>,
    powerline: &DelegationChain,
    grants: &[DelegationChain],
) -> Result<String> {
    let session = Ed25519Signer::import(&**seed)
        .await
        .map_err(|error| anyhow!("failed to derive the session key: {error:?}"))?;
    let did = session.did();
    ensure!(
        powerline.audience() == &did,
        "the powerline delegates to {}, not to the session key {did}",
        powerline.audience()
    );

    let mut meta = BTreeMap::from([
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
    if !grants.is_empty() {
        let mut encoded = Vec::with_capacity(grants.len());
        for grant in grants {
            ensure!(
                grant.audience() == powerline.issuer(),
                "a grant delegates to {}, not to the agent {}",
                grant.audience(),
                powerline.issuer()
            );
            encoded.push(Ipld::Bytes(
                grant
                    .to_bytes()
                    .map_err(|e| anyhow!("failed to encode a grant: {e}"))?,
            ));
        }
        meta.insert(TOKEN_GRANTS.to_owned(), Ipld::List(encoded));
    }
    let outer = DelegationBuilder::new()
        .issuer(Signer::from(session))
        .audience(&did)
        .subject(UcanSubject::Specific(did.clone()))
        .command(vec![])
        .meta(meta)
        .try_build()
        .await
        .map_err(|e| anyhow!("failed to mint the token's outer delegation: {e}"))?;
    let chain = powerline
        .push(outer)
        .map_err(|e| anyhow!("failed to wrap the powerline: {e}"))?;
    let encoded = Container::from(&chain)
        .encode(Tag::Base64Url)
        .map_err(|e| anyhow!("failed to encode the token: {e}"))?;
    String::from_utf8(encoded).context("a base64url container is ASCII")
}

/// An agent session token, decoded and checked.
pub struct AgentSessionToken {
    seed: Zeroizing<[u8; 32]>,
    session: Ed25519Signer,
    powerline: DelegationChain,
    home: Url,
    grants: Vec<DelegationChain>,
}

impl AgentSessionToken {
    /// The session key's seed. A host derives keys from it, such as the
    /// writer key of each MCP session; it never leaves the host.
    pub fn seed(&self) -> &Zeroizing<[u8; 32]> {
        &self.seed
    }

    /// The session key, derived from the token's seed.
    pub fn session(&self) -> &Ed25519Signer {
        &self.session
    }

    /// The session's DID: the operator of everything it signs.
    pub fn session_did(&self) -> Did {
        self.session.did()
    }

    /// The agent space's DID: the authority the session acts for.
    pub fn agent(&self) -> &Did {
        self.powerline.issuer()
    }

    /// The `agent space → session` powerline, without the token's outer
    /// delegation: the only proof an invocation needs, and the only part
    /// of the token without the seed in it.
    pub fn powerline(&self) -> &DelegationChain {
        &self.powerline
    }

    /// The agent's sync endpoint, from the powerline's signed `meta`.
    pub fn home(&self) -> &Url {
        &self.home
    }

    /// The grants the token carries, each delegating to [`Self::agent`].
    /// Decoding checks only their audience: whether each chain is valid,
    /// unexpired and unrevoked is for the verifier on every call.
    pub fn grants(&self) -> &[DelegationChain] {
        &self.grants
    }
}

/// Decode a token from [`encode_agent_session_token`] and check it.
///
/// Refuses anything but exactly a powerline under a self-issued outer
/// delegation that names a known type and version and carries a seed
/// whose key is that delegation's issuer and audience, both delegations
/// signed by their issuers. Whether the powerline has since been
/// revoked is not a property of the token: the verifier checks that on
/// every call.
pub async fn decode_agent_session_token(token: &str) -> Result<AgentSessionToken> {
    let container = Container::decode(token.trim().as_bytes())
        .map_err(|e| anyhow!("the token is not a UCAN container: {e}"))?;
    let chain = DelegationChain::try_from(container)
        .map_err(|e| anyhow!("the token is not a delegation chain: {e}"))?;
    let [powerline, outer] = chain.proofs().collect::<Vec<_>>()[..] else {
        bail!(
            "the token holds {} delegations; an agent session token holds two",
            chain.proof_cids().len()
        );
    };

    let meta = outer.meta();
    match meta.get(TOKEN_TYPE) {
        Some(Ipld::String(kind)) if kind == AGENT_SESSION_TOKEN => {}
        Some(Ipld::String(kind)) => bail!("unknown token type `{kind}`"),
        _ => bail!("the token names no `{TOKEN_TYPE}`"),
    }
    match meta.get(TOKEN_VERSION) {
        Some(Ipld::Integer(version)) if *version == i128::from(AGENT_SESSION_TOKEN_VERSION) => {}
        Some(Ipld::Integer(version)) => bail!("unsupported agent session token version {version}"),
        _ => bail!("the token names no `{TOKEN_VERSION}`"),
    }
    let Some(Ipld::Bytes(bytes)) = meta.get(TOKEN_SEED) else {
        bail!("the token carries no `{TOKEN_SEED}`");
    };
    let seed: Zeroizing<[u8; 32]> = Zeroizing::new(
        bytes
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("the token's seed is {} bytes, not 32", bytes.len()))?,
    );
    let session = Ed25519Signer::import(&*seed)
        .await
        .map_err(|error| anyhow!("failed to derive the session key: {error:?}"))?;
    let did = session.did();

    ensure!(
        outer.issuer() == &did && outer.audience() == &did,
        "the token's seed derives {did}, which did not issue it to itself"
    );
    ensure!(
        outer.subject() == &UcanSubject::Specific(did.clone()),
        "the token's outer delegation is about something other than its session"
    );
    ensure!(
        powerline.audience() == &did,
        "the token's powerline delegates to {}, not to its session",
        powerline.audience()
    );
    ensure!(
        powerline.subject() == &UcanSubject::Any && powerline.command().0.is_empty(),
        "the token's proof is not a powerline"
    );
    for delegation in [powerline, outer] {
        delegation
            .verify_signature(&DidKeyResolver)
            .await
            .map_err(|e| {
                anyhow!(
                    "the token's delegation from {} is not signed by it: {e}",
                    delegation.issuer()
                )
            })?;
    }

    let grants = match meta.get(TOKEN_GRANTS) {
        None => Vec::new(),
        Some(Ipld::List(items)) => items
            .iter()
            .map(|item| {
                let Ipld::Bytes(bytes) = item else {
                    bail!("a `{TOKEN_GRANTS}` entry is not bytes");
                };
                let grant = DelegationChain::try_from(bytes.as_slice()).map_err(|e| {
                    anyhow!("a `{TOKEN_GRANTS}` entry is not a delegation chain: {e}")
                })?;
                ensure!(
                    grant.audience() == powerline.issuer(),
                    "a grant delegates to {}, not to the agent {}",
                    grant.audience(),
                    powerline.issuer()
                );
                Ok(grant)
            })
            .collect::<Result<_>>()?,
        Some(_) => bail!("the token's `{TOKEN_GRANTS}` is not a list"),
    };

    let powerline = DelegationChain::new(powerline.clone());
    let home = home_address(&powerline)?.context("the token's powerline names no home")?;
    Ok(AgentSessionToken {
        seed,
        session,
        powerline,
        home,
        grants,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_ucan_core::{Environment, InvocationBuilder, InvocationChain};
    use dialog_ucan_core::{UnverifiedRevocations, VerificationContext};
    use std::collections::HashMap;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    const AGENT_SEED: [u8; 32] = [7u8; 32];
    const SESSION_SEED: [u8; 32] = [8u8; 32];
    const OTHER_SEED: [u8; 32] = [9u8; 32];

    async fn signer(seed: &[u8; 32]) -> Ed25519Signer {
        Ed25519Signer::import(seed).await.unwrap()
    }

    fn home() -> Url {
        Url::parse("https://tonk.example/").unwrap()
    }

    async fn token() -> (Ed25519Signer, String) {
        let agent = signer(&AGENT_SEED).await;
        let session = signer(&SESSION_SEED).await;
        let powerline = mint_agent_session(agent.clone(), &session.did(), &home())
            .await
            .unwrap();
        let token = encode_agent_session_token(&Zeroizing::new(SESSION_SEED), &powerline)
            .await
            .unwrap();
        (agent, token)
    }

    /// Re-wrap `powerline` under an outer delegation with `meta`, signed
    /// by `issuer` to `audience`: the tokens [`encode_agent_session_token`]
    /// would never write.
    async fn forged(
        powerline: &DelegationChain,
        issuer: Ed25519Signer,
        audience: &Did,
        meta: BTreeMap<String, Ipld>,
    ) -> String {
        let outer = DelegationBuilder::new()
            .issuer(Signer::from(issuer))
            .audience(audience)
            .subject(UcanSubject::Specific(audience.clone()))
            .command(vec![])
            .meta(meta)
            .try_build()
            .await
            .unwrap();
        let chain = powerline.push(outer).unwrap();
        String::from_utf8(Container::from(&chain).encode(Tag::Base64Url).unwrap()).unwrap()
    }

    fn meta(kind: &str, version: i128, seed: &[u8]) -> BTreeMap<String, Ipld> {
        BTreeMap::from([
            (TOKEN_TYPE.to_owned(), Ipld::String(kind.to_owned())),
            (TOKEN_VERSION.to_owned(), Ipld::Integer(version)),
            (TOKEN_SEED.to_owned(), Ipld::Bytes(seed.to_vec())),
        ])
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }

    #[dialog_common::test]
    async fn it_mints_an_unexpiring_powerline_naming_its_home() {
        let agent = signer(&AGENT_SEED).await;
        let session = signer(&SESSION_SEED).await;
        let powerline = mint_agent_session(agent.clone(), &session.did(), &home())
            .await
            .unwrap();

        assert_eq!(powerline.issuer(), &agent.did());
        assert_eq!(powerline.audience(), &session.did());
        assert!(powerline.subject().is_none(), "a powerline is subject-open");
        assert_eq!(powerline.ability(), "/");
        assert!(
            powerline.expiration().is_none(),
            "a session is revoked, never expired"
        );
        assert_eq!(home_address(&powerline).unwrap(), Some(home()));
    }

    #[dialog_common::test]
    async fn it_round_trips_a_token() {
        let (agent, token) = token().await;
        assert!(token.starts_with('C'), "the spec's base64url tag");
        assert!(
            token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "a token fits a header unescaped"
        );

        let decoded = decode_agent_session_token(&token).await.unwrap();
        assert_eq!(decoded.session_did(), signer(&SESSION_SEED).await.did());
        assert_eq!(decoded.agent(), &agent.did());
        assert_eq!(decoded.home(), &home());
        assert_eq!(decoded.powerline().proof_cids().len(), 1);
        assert_eq!(decoded.powerline().audience(), &decoded.session_did());
    }

    #[dialog_common::test]
    async fn it_issues_a_fresh_session_each_time() {
        let agent = signer(&AGENT_SEED).await;
        let first = issue_agent_session(agent.clone(), &home()).await.unwrap();
        let second = issue_agent_session(agent.clone(), &home()).await.unwrap();
        assert_ne!(first.powerline.audience(), second.powerline.audience());

        let decoded = decode_agent_session_token(&first.token).await.unwrap();
        assert_eq!(decoded.powerline(), &first.powerline);
        assert_eq!(decoded.agent(), &agent.did());
    }

    /// `space → device → agent` for each of the six agent-link scopes:
    /// the shape the approval page puts in a token for one space.
    async fn grants_for(space_seed: &[u8; 32], agent: &Did) -> Vec<DelegationChain> {
        let space = signer(space_seed).await;
        let device = signer(&[11u8; 32]).await;
        let mut chains = Vec::new();
        for verb in ["get", "put"] {
            for (kind, item) in [
                ("memory", "cell"),
                ("archive", "block"),
                ("archive", "blob"),
            ] {
                let command = vec!["use".into(), verb.into(), kind.into(), item.into()];
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
                    .command(command)
                    .meta(home_address_meta(&home()))
                    .try_build()
                    .await
                    .unwrap();
                chains.push(DelegationChain::new(root).push(leaf).unwrap());
            }
        }
        chains
    }

    #[dialog_common::test]
    async fn it_round_trips_a_token_with_grants() {
        let agent = signer(&AGENT_SEED).await;
        let mut grants = grants_for(&[21u8; 32], &agent.did()).await;
        grants.extend(grants_for(&[22u8; 32], &agent.did()).await);
        let issued = issue_agent_session_with_grants(agent.clone(), &home(), &grants)
            .await
            .unwrap();
        // Two spaces read/write must still fit an `Authorization` header.
        assert!(
            issued.token.len() < 16 * 1024,
            "{} bytes",
            issued.token.len()
        );

        let decoded = decode_agent_session_token(&issued.token).await.unwrap();
        assert_eq!(decoded.grants(), &grants[..]);
        assert_eq!(decoded.powerline(), &issued.powerline);

        let bare = issue_agent_session(agent, &home()).await.unwrap();
        let decoded = decode_agent_session_token(&bare.token).await.unwrap();
        assert!(decoded.grants().is_empty());
    }

    #[dialog_common::test]
    async fn it_refuses_a_grant_to_someone_else() {
        let agent = signer(&AGENT_SEED).await;
        let other = signer(&OTHER_SEED).await;
        let grants = grants_for(&[21u8; 32], &other.did()).await;
        assert!(
            issue_agent_session_with_grants(agent.clone(), &home(), &grants)
                .await
                .is_err()
        );

        let session = signer(&SESSION_SEED).await;
        let powerline = mint_agent_session(agent, &session.did(), &home())
            .await
            .unwrap();
        let mut smuggled = meta(AGENT_SESSION_TOKEN, 1, &SESSION_SEED);
        smuggled.insert(
            TOKEN_GRANTS.to_owned(),
            Ipld::List(vec![Ipld::Bytes(grants[0].to_bytes().unwrap())]),
        );
        let smuggled = forged(&powerline, session.clone(), &session.did(), smuggled).await;
        let error = decode_agent_session_token(&smuggled)
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("not to the agent"), "{error}");
    }

    #[dialog_common::test]
    async fn it_refuses_an_unknown_type_or_version() {
        let agent = signer(&AGENT_SEED).await;
        let session = signer(&SESSION_SEED).await;
        let powerline = mint_agent_session(agent, &session.did(), &home())
            .await
            .unwrap();

        let wrong_type = forged(
            &powerline,
            session.clone(),
            &session.did(),
            meta("device-session", 1, &SESSION_SEED),
        )
        .await;
        let error = decode_agent_session_token(&wrong_type)
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("unknown token type"), "{error}");

        let wrong_version = forged(
            &powerline,
            session.clone(),
            &session.did(),
            meta(AGENT_SESSION_TOKEN, 2, &SESSION_SEED),
        )
        .await;
        let error = decode_agent_session_token(&wrong_version)
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("unsupported"), "{error}");

        let mut untyped = meta(AGENT_SESSION_TOKEN, 1, &SESSION_SEED);
        untyped.remove(TOKEN_TYPE);
        let untyped = forged(&powerline, session.clone(), &session.did(), untyped).await;
        assert!(decode_agent_session_token(&untyped).await.is_err());
    }

    #[dialog_common::test]
    async fn it_refuses_a_seed_that_is_not_the_issuer_and_audience() {
        let agent = signer(&AGENT_SEED).await;
        let session = signer(&SESSION_SEED).await;
        let other = signer(&OTHER_SEED).await;
        let powerline = mint_agent_session(agent, &session.did(), &home())
            .await
            .unwrap();

        // A seed for some other key under a correctly self-issued outer.
        let wrong_seed = forged(
            &powerline,
            session.clone(),
            &session.did(),
            meta(AGENT_SESSION_TOKEN, 1, &OTHER_SEED),
        )
        .await;
        let error = decode_agent_session_token(&wrong_seed)
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("did not issue it to itself"), "{error}");

        // The right seed, but the outer is issued to someone else.
        let wrong_audience = forged(
            &powerline,
            session.clone(),
            &other.did(),
            meta(AGENT_SESSION_TOKEN, 1, &SESSION_SEED),
        )
        .await;
        let error = decode_agent_session_token(&wrong_audience)
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("did not issue it to itself"), "{error}");

        // A powerline to another key cannot be wrapped at all.
        let elsewhere = mint_agent_session(signer(&AGENT_SEED).await, &other.did(), &home())
            .await
            .unwrap();
        assert!(
            encode_agent_session_token(&Zeroizing::new(SESSION_SEED), &elsewhere)
                .await
                .is_err()
        );
    }

    #[dialog_common::test]
    async fn it_refuses_a_bare_powerline() {
        let agent = signer(&AGENT_SEED).await;
        let session = signer(&SESSION_SEED).await;
        let powerline = mint_agent_session(agent, &session.did(), &home())
            .await
            .unwrap();
        let bare =
            String::from_utf8(Container::from(&powerline).encode(Tag::Base64Url).unwrap()).unwrap();
        assert!(decode_agent_session_token(&bare).await.is_err());
        assert!(decode_agent_session_token("not a token").await.is_err());
    }

    #[dialog_common::test]
    async fn it_keeps_the_seed_out_of_the_powerline_and_invocations() {
        let (agent, token) = token().await;
        let decoded = decode_agent_session_token(&token).await.unwrap();

        // The token itself carries the seed: it is the credential.
        let raw = Container::decode(token.as_bytes())
            .unwrap()
            .into_bytes()
            .unwrap();
        assert!(contains(&raw, &SESSION_SEED));

        // What the agent space retains as the session row does not.
        let retained = decoded.powerline().to_bytes().unwrap();
        assert!(!contains(&retained, &SESSION_SEED));

        // Nor does what an invocation sends: the session signs over the
        // agent space with the powerline alone as proof, and that verifies.
        let invocation = InvocationBuilder::new()
            .issuer(Signer::from(decoded.session().clone()))
            .audience(decoded.agent())
            .subject(decoded.agent())
            .command(vec!["archive".to_owned(), "get".to_owned()])
            .arguments(BTreeMap::new())
            .proofs(decoded.powerline().proof_cids().to_vec())
            .try_build()
            .await
            .unwrap();
        let chain = InvocationChain::new(
            invocation,
            decoded.powerline().export().collect::<HashMap<_, _>>(),
        );
        let sent = chain.to_bytes().unwrap();
        assert!(!contains(&sent, &SESSION_SEED));

        let received = InvocationChain::try_from(sent.as_slice()).unwrap();
        assert_eq!(received.subject(), &agent.did());
        received
            .verify(&VerificationContext::new(&Environment::new(
                received.proof_store(),
                DidKeyResolver,
                UnverifiedRevocations,
            )))
            .await
            .unwrap();
    }
}
