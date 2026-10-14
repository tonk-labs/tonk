//! The native space purger against a real local S3 server.
//!
//! Pins the path-style URL shape: `Address::resolve` prepends
//! `{bucket}/` to the request path, so the bucket listing must use the
//! empty path — a `"/"` produced `{bucket}//`, which S3 reads as an
//! object named `"/"` and answers 404, leaving every hosted-space
//! deletion "temporarily incomplete".
#![cfg(all(feature = "helpers", not(target_arch = "wasm32")))]

use dialog_common::helpers::Provider as _;
use dialog_remote_s3::Address;
use dialog_remote_s3::helpers::LocalS3;
use dialog_remote_s3::request::S3Request;
use dialog_remote_s3::s3::S3Credential;
use tonk_access_service::deletion::{NativeSpacePurger, SpacePurger};

const ACCESS_KEY: &str = "purger-access-key";
const SECRET_KEY: &str = "purger-secret-key";
const BUCKET: &str = "purger-bucket";

async fn request(
    credential: &S3Credential,
    address: &Address,
    method: &str,
    path: &str,
    params: Option<Vec<(String, String)>>,
    body: Option<Vec<u8>>,
) -> anyhow::Result<reqwest::Response> {
    let permit = S3Request {
        method: method.to_string(),
        path: path.to_string(),
        params,
        ..Default::default()
    }
    .attest(credential.clone())
    .redeem(address)
    .await
    .map_err(|error| anyhow::anyhow!("presign {method} {path}: {error:?}"))?;
    let client = reqwest::Client::new();
    let mut builder = match method {
        "PUT" => client.put(permit.url),
        "DELETE" => client.delete(permit.url),
        _ => client.get(permit.url),
    };
    for (name, value) in &permit.headers {
        builder = builder.header(name, value);
    }
    if let Some(body) = body {
        builder = builder.body(body);
    }
    Ok(builder.send().await?.error_for_status()?)
}

async fn remaining_keys(credential: &S3Credential, address: &Address) -> anyhow::Result<String> {
    let listed = request(
        credential,
        address,
        "GET",
        "",
        Some(vec![("list-type".to_string(), "2".to_string())]),
        None,
    )
    .await?;
    Ok(listed.text().await?)
}

#[dialog_common::test]
async fn it_purges_exactly_the_space_prefix() -> anyhow::Result<()> {
    let server = LocalS3::start_with_auth(ACCESS_KEY, SECRET_KEY, &[BUCKET]).await?;
    let address = Address::builder(&server.endpoint)
        .region("us-east-1")
        .bucket(BUCKET)
        .path_style(true)
        .build()?;
    let credential = S3Credential::new(ACCESS_KEY, SECRET_KEY);

    let doomed = "did:key:zDoomedSpace";
    let neighbor = "did:key:zNeighborSpace";
    for key in [
        format!("{doomed}/blocks/one"),
        format!("{doomed}/blocks/two"),
        format!("{neighbor}/blocks/keep"),
    ] {
        request(
            &credential,
            &address,
            "PUT",
            &key,
            None,
            Some(b"data".to_vec()),
        )
        .await?;
    }

    let purger = NativeSpacePurger::new(address.clone(), credential.clone());
    purger
        .purge(&format!("{doomed}/"))
        .await
        .map_err(|error| anyhow::anyhow!("purge failed: {error}"))?;

    let listing = remaining_keys(&credential, &address).await?;
    assert!(
        !listing.contains(doomed),
        "purged space keys survived: {listing}"
    );
    assert!(
        listing.contains(neighbor),
        "the purge crossed into another space's prefix: {listing}"
    );

    server.stop().await?;
    Ok(())
}
