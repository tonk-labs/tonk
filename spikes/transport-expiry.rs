//! Ordinary signed transport regression, registered in an isolated snapshot by
//! scripts/test-transport-expiry.sh. No connection policy or management state.
use dialog_credentials::Ed25519Signer;
use dialog_remote_s3::{Address, S3Credential, helpers::LocalS3};
use dialog_remote_ucan_s3::UcanAuthorizer;
use dialog_ucan_core::{
    DelegationBuilder, InvocationBuilder, InvocationChain,
    promise::Promised,
    subject::Subject,
    time::{Duration, SystemTime, Timestamp},
};
use dialog_varsig::Principal;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

#[tokio::test]
async fn transport_expiry_signed_read_write_preserves_headers() -> anyhow::Result<()> {
    let s3 = LocalS3::start_with_auth("test", "test", &["transport"]).await?;
    let address = Address::builder(&s3.endpoint)
        .region("us-east-1")
        .bucket("transport")
        .path_style(true)
        .build()?;
    let authorizer = UcanAuthorizer::new(address, Some(S3Credential::new("test", "test")));
    let owner = Ed25519Signer::import(&[81; 32]).await?;
    let agent = Ed25519Signer::import(&[82; 32]).await?;
    let content = b"ordinary scoped transport";
    let deadline = Timestamp::new(SystemTime::now() + Duration::from_secs(120))?;
    let client = reqwest::Client::new();
    for operation in ["put", "get"] {
        let command = vec![
            "use".into(),
            operation.into(),
            "archive".into(),
            "block".into(),
        ];
        let grant = DelegationBuilder::new()
            .issuer(owner.clone())
            .audience(&agent.did())
            .subject(Subject::Specific(owner.did()))
            .command(command.clone())
            .expiration(deadline)
            .try_build()
            .await?;
        let checksum = dialog_common::Hasher::Sha256.checksum(content);
        let checksum: Promised =
            serde_ipld_dagcbor::from_slice(&serde_ipld_dagcbor::to_vec(&checksum)?)?;
        let invocation = InvocationBuilder::new()
            .issuer(agent.clone())
            .audience(&owner.did())
            .subject(&owner.did())
            .command(command)
            .arguments(BTreeMap::from([
                ("catalog".into(), Promised::String("index".into())),
                (
                    "digest".into(),
                    Promised::Bytes(blake3::hash(content).as_bytes().to_vec()),
                ),
                ("checksum".into(), checksum),
            ]))
            .proofs(vec![grant.to_cid()])
            .try_build()
            .await?;
        let bytes = InvocationChain::new(
            invocation,
            HashMap::from([(grant.to_cid(), Arc::new(grant))]),
        )
        .to_bytes()?;
        let permit = authorizer.authorize(&bytes).await?;
        let ttl: u64 = permit
            .url
            .query_pairs()
            .find(|(k, _)| k == "X-Amz-Expires")
            .unwrap()
            .1
            .parse()?;
        assert!((1..=60).contains(&ttl));
        let mut request = client.request(permit.method.parse()?, permit.url.clone());
        for (name, value) in &permit.headers {
            request = request.header(name, value);
        }
        if operation == "put" {
            assert!(
                permit
                    .headers
                    .iter()
                    .any(|(name, _)| name.to_ascii_lowercase().contains("checksum"))
            );
            request
                .body(content.to_vec())
                .send()
                .await?
                .error_for_status()?;
        } else {
            assert_eq!(
                request
                    .send()
                    .await?
                    .error_for_status()?
                    .bytes()
                    .await?
                    .as_ref(),
                content
            );
        }
    }
    Ok(())
}
