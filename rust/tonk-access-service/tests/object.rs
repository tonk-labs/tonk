//! The object path: where a `/ucan/` answer is redeemed.
//!
//! These drive `/object/{key}` directly, minting permits under the
//! service seed the harness exposes, so every answer the handler gives
//! is pinned: the bytes come back, a range comes back partial, a
//! checksum or precondition that does not hold is refused, and a permit
//! is worth nothing outside exactly what it names. The full flow, from
//! invocation to bytes, is covered by `ucan_integration`.
//!
//! Run with:
//! ```bash
//! cargo test -p tonk-access-service --features integration-tests --test object
//! ```

#![cfg(feature = "integration-tests")]

use dialog_remote_s3::Permit;
use reqwest::StatusCode;
use sha2_0_10::{Digest as _, Sha256};
use tonk_access_service::helpers::AccessServiceAddress;
use tonk_access_service::permit::{Claims, Method, PERMIT_TTL, PermitKey, Precondition};

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the clock is past the epoch")
        .as_secs()
}

fn permit_key(env: &AccessServiceAddress) -> PermitKey {
    PermitKey::derive(&env.service_seed).expect("the harness seed derives a key")
}

fn claims(method: Method, key: &str) -> Claims {
    Claims {
        method,
        key: key.to_string(),
        expires: now() + PERMIT_TTL,
        sha256: None,
        precondition: Precondition::None,
    }
}

fn write(key: &str, body: &[u8]) -> Claims {
    Claims {
        sha256: Some(Sha256::digest(body).to_vec()),
        ..claims(Method::Put, key)
    }
}

fn issue(env: &AccessServiceAddress, claims: &Claims) -> Permit {
    permit_key(env)
        .issue(&env.access_service_url, claims)
        .expect("a permit issues")
}

/// Present `permit` the way the client does: the URL and method it
/// carries, nothing else.
async fn present(permit: Permit, body: Option<Vec<u8>>) -> reqwest::Response {
    let mut request = reqwest::RequestBuilder::from(permit);
    if let Some(body) = body {
        request = request.body(body);
    }
    request.send().await.expect("the service answers")
}

fn object(name: &str) -> String {
    format!("did:key:z6MkObjectTest/index/{name}-{}", rand_suffix())
}

fn rand_suffix() -> String {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("entropy");
    hex::encode(bytes)
}

#[dialog_common::test]
async fn it_stores_and_serves_a_permitted_object(env: AccessServiceAddress) {
    let key = object("block");
    let body = b"the block's bytes".to_vec();

    let stored = present(issue(&env, &write(&key, &body)), Some(body.clone())).await;
    assert_eq!(stored.status(), StatusCode::OK);
    assert!(
        stored.headers().get("etag").is_some(),
        "a write answers with the stored version"
    );

    let read = present(issue(&env, &claims(Method::Get, &key)), None).await;
    assert_eq!(read.status(), StatusCode::OK);
    assert!(read.headers().get("etag").is_some());
    assert_eq!(read.bytes().await.unwrap().as_ref(), body.as_slice());
}

#[dialog_common::test]
async fn it_serves_a_byte_range(env: AccessServiceAddress) {
    let key = object("blob");
    let body = b"0123456789".to_vec();
    present(issue(&env, &write(&key, &body)), Some(body)).await;

    let permit = issue(&env, &claims(Method::Get, &key));
    let read = reqwest::RequestBuilder::from(permit)
        .header("range", "bytes=2-5")
        .send()
        .await
        .unwrap();
    assert_eq!(read.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(read.bytes().await.unwrap().as_ref(), b"2345");
}

#[dialog_common::test]
async fn it_answers_an_absent_object_with_not_found(env: AccessServiceAddress) {
    let read = present(issue(&env, &claims(Method::Get, &object("absent"))), None).await;
    assert_eq!(read.status(), StatusCode::NOT_FOUND);
}

/// The checksum is bound at authorization time, so a holder of a write
/// permit cannot store anything but the content it was issued for.
#[dialog_common::test]
async fn it_refuses_a_body_the_permit_did_not_bind(env: AccessServiceAddress) {
    let key = object("block");
    let permit = issue(&env, &write(&key, b"the authorized bytes"));

    let stored = present(permit, Some(b"something else".to_vec())).await;
    assert_eq!(stored.status(), StatusCode::BAD_REQUEST);

    let read = present(issue(&env, &claims(Method::Get, &key)), None).await;
    assert_eq!(read.status(), StatusCode::NOT_FOUND, "nothing was stored");
}

/// A cell publish is a compare-and-set: create-only for a fresh cell,
/// then conditional on the version last read.
#[dialog_common::test]
async fn it_enforces_the_write_precondition(env: AccessServiceAddress) {
    let key = object("cell");
    let first = b"first edition".to_vec();
    let create = Claims {
        precondition: Precondition::IfNoneMatch,
        ..write(&key, &first)
    };

    let created = present(issue(&env, &create), Some(first.clone())).await;
    assert_eq!(created.status(), StatusCode::OK);
    let version = created.headers()["etag"]
        .to_str()
        .unwrap()
        .trim_matches('"')
        .to_string();

    let again = present(issue(&env, &create), Some(first)).await;
    assert_eq!(
        again.status(),
        StatusCode::PRECONDITION_FAILED,
        "create-only refuses an existing cell"
    );

    let second = b"second edition".to_vec();
    let stale = present(
        issue(
            &env,
            &Claims {
                precondition: Precondition::IfMatch("not-the-version".to_string()),
                ..write(&key, &second)
            },
        ),
        Some(second.clone()),
    )
    .await;
    assert_eq!(stale.status(), StatusCode::PRECONDITION_FAILED);

    let current = present(
        issue(
            &env,
            &Claims {
                precondition: Precondition::IfMatch(version),
                ..write(&key, &second)
            },
        ),
        Some(second.clone()),
    )
    .await;
    assert_eq!(current.status(), StatusCode::OK);

    let read = present(issue(&env, &claims(Method::Get, &key)), None).await;
    assert_eq!(read.bytes().await.unwrap().as_ref(), second.as_slice());
}

#[dialog_common::test]
async fn it_deletes_a_permitted_object(env: AccessServiceAddress) {
    let key = object("cell");
    let body = b"to be removed".to_vec();
    present(issue(&env, &write(&key, &body)), Some(body)).await;

    let removed = present(issue(&env, &claims(Method::Delete, &key)), None).await;
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);

    let read = present(issue(&env, &claims(Method::Get, &key)), None).await;
    assert_eq!(read.status(), StatusCode::NOT_FOUND);
}

#[dialog_common::test]
async fn it_refuses_a_lapsed_permit(env: AccessServiceAddress) {
    let lapsed = Claims {
        expires: now() - 1,
        ..claims(Method::Get, &object("block"))
    };
    let read = present(issue(&env, &lapsed), None).await;
    assert_eq!(read.status(), StatusCode::UNAUTHORIZED);
}

#[dialog_common::test]
async fn it_refuses_a_permit_not_signed_by_the_service(env: AccessServiceAddress) {
    let other = PermitKey::derive(&"42".repeat(32)).unwrap();
    let permit = other
        .issue(
            &env.access_service_url,
            &claims(Method::Get, &object("block")),
        )
        .unwrap();
    let read = present(permit, None).await;
    assert_eq!(read.status(), StatusCode::UNAUTHORIZED);

    let bare = reqwest::get(format!(
        "{}/object/{}",
        env.access_service_url.trim_end_matches('/'),
        object("block")
    ))
    .await
    .unwrap();
    assert_eq!(bare.status(), StatusCode::BAD_REQUEST, "no permit at all");
}

/// A genuine permit is worth exactly what it names: not another
/// object, and not another operation on the same one.
#[dialog_common::test]
async fn it_refuses_a_permit_beyond_what_it_names(env: AccessServiceAddress) {
    let mine = object("mine");
    let theirs = object("theirs");
    let body = b"their bytes".to_vec();
    present(issue(&env, &write(&theirs, &body)), Some(body)).await;

    // The token for `mine`, presented at `theirs`.
    let mut permit = issue(&env, &claims(Method::Get, &mine));
    let query = permit.url.query().unwrap().to_string();
    permit.url.set_path(&format!("/object/{theirs}"));
    permit.url.set_query(Some(&query));
    let read = present(permit, None).await;
    assert_eq!(read.status(), StatusCode::FORBIDDEN);

    // A read permit for `theirs`, used as a write.
    let mut permit = issue(&env, &claims(Method::Get, &theirs));
    permit.method = "PUT".to_string();
    let written = present(permit, Some(b"overwritten".to_vec())).await;
    assert_eq!(written.status(), StatusCode::FORBIDDEN);

    // A read permit for `theirs`, used as a delete.
    let mut permit = issue(&env, &claims(Method::Get, &theirs));
    permit.method = "DELETE".to_string();
    let deleted = present(permit, None).await;
    assert_eq!(deleted.status(), StatusCode::FORBIDDEN);

    let read = present(issue(&env, &claims(Method::Get, &theirs)), None).await;
    assert_eq!(read.bytes().await.unwrap().as_ref(), b"their bytes");
}
