//! Addressed additions to an established terminal. Initial request expiry does
//! not extend or constrain these independently issued ordinary grants.
use super::*;

const READ_ADDITIONS_DOMAIN: &[u8] = b"tonk/terminal-read-additions/v1\0";
type AdditionPayload = (
    u64,
    String,
    String,
    String,
    String,
    String,
    ByteBuf,
    u64,
    ByteBuf,
    Vec<BundlePayload>,
);
type ReadAdditionsPayload = (u64, String, String, u64, ByteBuf, u64, u64);

/// Complete public grant addition, signed by a currently authorized device.
#[derive(Debug, Clone)]
pub struct Addition {
    bytes: Vec<u8>,
    request_id: String,
    recipient: Did,
    account: Did,
    service: Did,
    issuer: Did,
    issued_at: u64,
    bundles: Vec<SpaceGrantBundle>,
    authorization: InvocationChain<AnySignature>,
}

impl Addition {
    /// Verify historical signed issuance for management. Current publication
    /// and grant use must still call `validate` and check revocations.
    pub async fn inspect(bytes: &[u8]) -> Result<Self> {
        let (payload, _): Envelope = decode(bytes, MAX_APPROVAL_BYTES)?;
        let (_, _, _, _, _, _, _, issued_at, _, _): AdditionPayload =
            decode(&payload, MAX_APPROVAL_BYTES)?;
        Self::validate(bytes, issued_at).await
    }
    /// Derive immutable delivery pins from a verified initial approval. The
    /// caller may inspect that approval historically; only `account_proof`
    /// supplies authority for this new issuance.
    pub async fn sign(
        signer: &Signer,
        initial: &Approval,
        account_proof: DelegationChain,
        bundles: Vec<SpaceGrantBundle>,
        nonce: [u8; 32],
        now: u64,
    ) -> Result<Self> {
        ensure!(!initial.is_declined(), "terminal_initial_request_declined");
        let mut selected = Vec::new();
        for bundle in bundles {
            let cids: Vec<String> = bundle
                .chains()
                .iter()
                .map(|c| {
                    c.proofs()
                        .last()
                        .expect("validated chain")
                        .to_cid()
                        .to_string()
                })
                .collect();
            selected.push((
                bundle.subject().to_string(),
                bundle.remote().to_string(),
                grant_set_id(
                    bundle.subject().as_str(),
                    initial.request().recipient().as_str(),
                    &cids,
                ),
                bundle
                    .chains()
                    .iter()
                    .map(|c| c.to_bytes().map(ByteBuf::from))
                    .collect::<std::result::Result<Vec<_>, _>>()?,
            ));
        }
        selected.sort_by(|a, b| a.0.cmp(&b.0));
        let payload = encode(&(
            1,
            initial.request().id(),
            initial.request().recipient().to_string(),
            initial.account().to_string(),
            initial.request().service().to_string(),
            signer.did().to_string(),
            ByteBuf::from(nonce.to_vec()),
            now,
            ByteBuf::from(account_proof.to_bytes()?),
            selected,
        ))?;
        let invocation = InvocationBuilder::new()
            .issuer(signer.clone())
            .audience(initial.request().service())
            .subject(initial.account())
            .command(vec!["connection".into(), "addition".into()])
            .arguments(BTreeMap::from([(
                "payload".into(),
                Promised::String(blake3::hash(&payload).to_hex().to_string()),
            )]))
            .proofs(account_proof.proof_cids().to_vec())
            .issued_at(timestamp(now)?)
            .expiration(
                account_proof.expiration().unwrap_or(timestamp(
                    now.checked_add(crate::connection::DEFAULT_GRANT_TTL_SECONDS)
                        .context("terminal_invalid_time")?,
                )?),
            )
            .try_build()
            .await
            .context("terminal_signing_failed")?;
        let mut tokens = vec![encode(&invocation)?];
        tokens.extend(account_proof.export().map(|(_, p)| p.encoded().to_vec()));
        let bytes = encode(&(
            ByteBuf::from(payload),
            ByteBuf::from(Container::new(tokens).into_bytes()?),
        ))?;
        Self::validate(&bytes, now).await
    }

    /// Verify exact public payload, rooted current account proof, recipient and
    /// grants. Service callers additionally check the original mailbox pins,
    /// configured audience and revocations with the standard UCAN verifier.
    pub async fn validate(bytes: &[u8], now: u64) -> Result<Self> {
        let (payload, authorization): Envelope = decode(bytes, MAX_APPROVAL_BYTES)?;
        let (
            version,
            request_id,
            recipient,
            account,
            service,
            issuer,
            nonce,
            issued_at,
            proof,
            selected,
        ): AdditionPayload = decode(&payload, MAX_APPROVAL_BYTES)?;
        ensure!(version == 1, "terminal_unsupported_version");
        identifier(&request_id)?;
        ensure!(
            nonce.len() == 32 && issued_at <= now.saturating_add(30),
            "terminal_invalid_addition_time"
        );
        ensure!(
            !selected.is_empty() && selected.len() <= MAX_SELECTED_SPACES,
            "terminal_invalid_selection"
        );
        let recipient = ed25519(&recipient)?.did();
        let account = ed25519(&account)?.did();
        let service = ed25519(&service)?.did();
        let issuer = ed25519(&issuer)?.did();
        ensure!(
            recipient != account && recipient != issuer,
            "terminal_account_authority_addressed_to_recipient"
        );
        let proof =
            DelegationChain::try_from(proof.as_ref()).context("terminal_invalid_account_proof")?;
        ensure!(
            proof.issuer() == &account && proof.audience() == &issuer,
            "terminal_wrong_account"
        );
        check_chain(proof.proofs(), &account, Some(timestamp(now)?))
            .context("terminal_invalid_account_proof")?;
        for hop in proof.proofs() {
            ensure!(
                hop.issuer() != &recipient
                    && hop.audience() != &recipient
                    && hop.command().0.is_empty()
                    && hop.policy().is_empty(),
                "terminal_invalid_account_proof"
            );
            hop.verify_signature(&DidKeyResolver)
                .await
                .context("terminal_invalid_account_proof")?;
        }
        let authorization = InvocationChain::<AnySignature>::try_from(authorization.as_ref())
            .context("terminal_invalid_authorization")?;
        ensure!(
            authorization.issuer() == &issuer
                && authorization.subject() == &account
                && authorization.invocation.audience() == &service
                && authorization.command().0 == ["connection", "addition"]
                && authorization.arguments()
                    == &BTreeMap::from([(
                        "payload".into(),
                        Promised::String(blake3::hash(&payload).to_hex().to_string())
                    )])
                && authorization.proofs() == proof.proof_cids(),
            "terminal_authorization_mismatch"
        );
        ensure!(
            authorization
                .invocation
                .expiration()
                .is_some_and(|t| t > timestamp(now).expect("validated time")),
            "terminal_authorization_expired"
        );
        authorization
            .invocation
            .verify_signature(&DidKeyResolver)
            .await
            .context("terminal_invalid_authorization")?;
        let mut bundles = Vec::new();
        let mut previous = None;
        for (subject, remote, id, chains) in selected {
            ensure!(
                previous.as_ref().is_none_or(|p: &String| p < &subject),
                "terminal_duplicate_or_unsorted_selection"
            );
            previous = Some(subject.clone());
            let subject: Did = subject.parse().context("terminal_invalid_subject")?;
            ensure!(subject != account, "terminal_account_catalogue_forbidden");
            let remote = Url::parse(&remote).context("terminal_invalid_route")?;
            endpoint(&remote)?;
            let chains = chains
                .into_iter()
                .map(|c| DelegationChain::try_from(c.as_ref()).context("terminal_invalid_grants"))
                .collect::<Result<Vec<_>>>()?;
            let bundle = SpaceGrantBundle::validate(
                chains,
                &recipient,
                &candidate_build_scopes(&subject),
                &remote,
                timestamp(now)?,
            )
            .await?;
            let cids: Vec<String> = bundle
                .chains()
                .iter()
                .map(|c| {
                    c.proofs()
                        .last()
                        .expect("validated chain")
                        .to_cid()
                        .to_string()
                })
                .collect();
            ensure!(
                id == grant_set_id(subject.as_str(), recipient.as_str(), &cids),
                "terminal_group_mismatch"
            );
            bundles.push(bundle);
        }
        Ok(Self {
            bytes: bytes.to_vec(),
            request_id,
            recipient,
            account,
            service,
            issuer,
            issued_at,
            bundles,
            authorization,
        })
    }
    /// Exact complete signed addition bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// Content digest used for immutable delivery replay.
    pub fn id(&self) -> String {
        blake3::hash(&self.bytes).to_hex().to_string()
    }
    /// Established terminal request hash.
    pub fn request_id(&self) -> &str {
        &self.request_id
    }
    /// Exact CLI key addressed by every included grant.
    pub fn recipient(&self) -> &Did {
        &self.recipient
    }
    /// Account proven by the current signed invocation.
    pub fn account(&self) -> &Did {
        &self.account
    }
    /// Service audience fixed by the original request.
    pub fn service(&self) -> &Did {
        &self.service
    }
    /// Device signing this addition.
    pub fn issuer(&self) -> &Did {
        &self.issuer
    }
    /// Signed issuance time.
    pub fn issued_at(&self) -> u64 {
        self.issued_at
    }
    /// Complete sorted selected-space grants.
    pub fn bundles(&self) -> &[SpaceGrantBundle] {
        &self.bundles
    }
    /// Standard invocation for service-side revocation enforcement.
    pub fn authorization(&self) -> &InvocationChain<AnySignature> {
        &self.authorization
    }
}

/// Signed exact-recipient read with a bound cursor. Cursor state never grants
/// authority and the request can be retried without consuming a capability.
#[derive(Debug, Clone)]
pub struct ReadAdditions {
    bytes: Vec<u8>,
    request_id: String,
    recipient: Did,
    after: u64,
}
impl ReadAdditions {
    /// Sign an exact mailbox and cursor without persistent pending state.
    pub async fn sign(
        signer: &Signer,
        request_id: &str,
        after: u64,
        nonce: [u8; 32],
        now: u64,
    ) -> Result<Self> {
        let bytes = sign(
            signer,
            READ_ADDITIONS_DOMAIN,
            &(
                1,
                request_id,
                signer.did().to_string(),
                after,
                ByteBuf::from(nonce.to_vec()),
                now,
                now.checked_add(READ_TTL_SECONDS)
                    .context("terminal_invalid_time")?,
            ),
        )
        .await?;
        Self::validate(&bytes, now).await
    }
    /// Verify recipient possession, cursor binding and the short read window.
    pub async fn validate(bytes: &[u8], now: u64) -> Result<Self> {
        let (payload, _): Envelope = decode(bytes, MAX_REQUEST_BYTES)?;
        let (version, request_id, recipient, after, nonce, created, expires): ReadAdditionsPayload =
            decode(&payload, MAX_REQUEST_BYTES)?;
        ensure!(version == 1, "terminal_unsupported_version");
        identifier(&request_id)?;
        ensure!(
            after <= 9_007_199_254_740_991
                && nonce.len() == 32
                && expires > created
                && expires - created <= READ_TTL_SECONDS
                && created <= now.saturating_add(30)
                && expires > now,
            "terminal_invalid_read_window"
        );
        verify(bytes, MAX_REQUEST_BYTES, READ_ADDITIONS_DOMAIN, &recipient).await?;
        Ok(Self {
            bytes: bytes.to_vec(),
            request_id,
            recipient: ed25519(&recipient)?.did(),
            after,
        })
    }
    /// Exact signed read bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// Established terminal request hash.
    pub fn request_id(&self) -> &str {
        &self.request_id
    }
    /// The key which authenticated this read.
    pub fn recipient(&self) -> &Did {
        &self.recipient
    }
    /// Exclusive transport cursor, never an authority version.
    pub fn after(&self) -> u64 {
        self.after
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn grants(recipient: &Did, expires: u64, now: u64) -> Result<SpaceGrantBundle> {
        let root = Signer::from(dialog_credentials::Ed25519Signer::generate().await?);
        grants_from(&root, recipient, expires, now).await
    }
    async fn grants_from(
        root: &Signer,
        recipient: &Did,
        expires: u64,
        now: u64,
    ) -> Result<SpaceGrantBundle> {
        use dialog_ucan_core::{DelegationBuilder, subject::Subject};
        let remote: Url = "https://sync.example.test/ucan/".parse()?;
        let scopes = candidate_build_scopes(&root.did());
        let mut chains = Vec::new();
        for scope in &scopes {
            let grant = DelegationBuilder::new()
                .issuer(root.clone())
                .audience(recipient)
                .subject(Subject::Specific(root.did()))
                .command(scope.command.0.clone())
                .policy(scope.policy())
                .expiration(timestamp(expires)?)
                .meta(BTreeMap::from([(
                    "home.address".into(),
                    ipld_core::ipld::Ipld::String(remote.to_string()),
                )]))
                .try_build()
                .await?;
            chains.push(DelegationChain::new(grant));
        }
        SpaceGrantBundle::validate(chains, recipient, &scopes, &remote, timestamp(now)?).await
    }

    #[tokio::test]
    async fn terminal_addition_and_initial_refuse_account_catalogue_grants() -> Result<()> {
        use dialog_ucan_core::{DelegationBuilder, subject::Subject};
        let account = Signer::from(dialog_credentials::Ed25519Signer::generate().await?);
        let browser = Signer::from(dialog_credentials::Ed25519Signer::generate().await?);
        let cli = Signer::from(dialog_credentials::Ed25519Signer::generate().await?);
        let service = Signer::from(dialog_credentials::Ed25519Signer::generate().await?);
        let proof = DelegationChain::new(
            DelegationBuilder::new()
                .issuer(account.clone())
                .audience(&browser.did())
                .subject(Subject::Any)
                .command(vec![])
                .try_build()
                .await?,
        );
        let request =
            LinkRequest::sign(&cli, &service.did(), [5; 32], 1000, 1040, "terminal", None).await?;
        let catalogue = grants_from(&account, &cli.did(), 1100, 1000).await?;
        let initial_attempt = Approval::sign(
            &browser,
            &request,
            proof.clone(),
            vec![catalogue.clone()],
            1000,
        )
        .await;
        let initial = Approval::sign(
            &browser,
            &request,
            proof.clone(),
            vec![grants(&cli.did(), 1100, 1000).await?],
            1000,
        )
        .await?;
        let addition_attempt =
            Addition::sign(&browser, &initial, proof, vec![catalogue], [6; 32], 1000).await;
        assert!(
            initial_attempt.is_err() && addition_attempt.is_err(),
            "account catalogue grant accepted: initial={}, addition={}",
            initial_attempt.is_ok(),
            addition_attempt.is_ok()
        );
        Ok(())
    }

    #[tokio::test]
    async fn terminal_addition_uses_new_device_after_initial_authority_and_grants_expire()
    -> Result<()> {
        use dialog_ucan_core::{DelegationBuilder, subject::Subject};
        let account = Signer::from(dialog_credentials::Ed25519Signer::generate().await?);
        let old = Signer::from(dialog_credentials::Ed25519Signer::generate().await?);
        let current = Signer::from(dialog_credentials::Ed25519Signer::generate().await?);
        let cli = Signer::from(dialog_credentials::Ed25519Signer::generate().await?);
        let service = Signer::from(dialog_credentials::Ed25519Signer::generate().await?);
        let old_proof = DelegationChain::new(
            DelegationBuilder::new()
                .issuer(account.clone())
                .audience(&old.did())
                .subject(Subject::Any)
                .command(vec![])
                .expiration(timestamp(1050)?)
                .try_build()
                .await?,
        );
        let request = LinkRequest::sign(
            &cli,
            &service.did(),
            [2; 32],
            1000,
            1040,
            "terminal",
            Some(&account.did()),
        )
        .await?;
        let initial = Approval::sign(
            &old,
            &request,
            old_proof.clone(),
            vec![grants(&cli.did(), 1060, 1000).await?],
            1000,
        )
        .await?;
        assert!(Approval::validate(initial.bytes(), 1200).await.is_err());
        let historical = Approval::inspect(initial.bytes()).await?;
        let proof = DelegationChain::new(
            DelegationBuilder::new()
                .issuer(account)
                .audience(&current.did())
                .subject(Subject::Any)
                .command(vec![])
                .try_build()
                .await?,
        );
        let fresh = grants(&cli.did(), 1300, 1200).await?;
        assert!(
            Addition::sign(
                &old,
                &historical,
                old_proof,
                vec![fresh.clone()],
                [3; 32],
                1200
            )
            .await
            .is_err()
        );
        let addition =
            Addition::sign(&current, &historical, proof, vec![fresh], [4; 32], 1200).await?;
        assert_eq!(addition.account(), initial.account());
        assert_eq!(addition.issuer(), &current.did());
        assert!(Addition::validate(addition.bytes(), 1310).await.is_err());
        assert_eq!(
            Addition::inspect(addition.bytes()).await?.id(),
            addition.id()
        );
        Ok(())
    }
    #[tokio::test]
    async fn terminal_addition_read_binds_cursor_domain_and_window() -> Result<()> {
        let signer = Signer::from(dialog_credentials::Ed25519Signer::generate().await?);
        let id = "ab".repeat(32);
        let request = ReadAdditions::sign(&signer, &id, 17, [1; 32], 1000).await?;
        assert_eq!(
            ReadAdditions::validate(request.bytes(), 1001)
                .await?
                .after(),
            17
        );
        assert!(
            ReadAdditions::validate(request.bytes(), 1060)
                .await
                .is_err()
        );
        assert!(ReadRequest::validate(request.bytes(), 1001).await.is_err());
        let (payload, signature): Envelope = decode(request.bytes(), MAX_REQUEST_BYTES)?;
        let mut fields: ReadAdditionsPayload = decode(&payload, MAX_REQUEST_BYTES)?;
        fields.3 = 18;
        let changed = encode(&(ByteBuf::from(encode(&fields)?), signature))?;
        assert!(ReadAdditions::validate(&changed, 1001).await.is_err());
        Ok(())
    }
}
