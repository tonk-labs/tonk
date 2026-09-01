//! Lookup of a customer by email address, end to end against the running
//! service.
//!
//! The unit tests in `tonk_access_service::lookup` cover the DID encoding
//! in isolation. These cover what only the wired service can answer: that
//! the address a caller holds finds the row enrollment wrote, that the
//! registration state picks the status code, and that the path survives
//! the segments `did:web` resolution hands over.
//!
//! Run with:
//! ```bash
//! cargo test -p tonk-access-service --features integration-tests --test lookup
//! ```

#![cfg(feature = "integration-tests")]

use dialog_credentials::Ed25519Signer;
use dialog_varsig::{Did, Principal};
use tonk_access_service::helpers::AccessServiceAddress;
use tonk_access_service::lookup::customer_did;

/// The host the service answers as, which is the host its `did:web` names
/// are minted under.
fn host(env: &AccessServiceAddress) -> String {
    url::Url::parse(&env.access_service_url)
        .expect("the service url parses")
        .authority()
        .to_string()
}

/// The service host as it appears INSIDE a `did:web`.
///
/// `did:web` separates path segments with `:`, so a host carrying a port
/// percent-encodes it. The tests run against an ephemeral port, so every
/// DID they build or take apart carries one -- spelled raw here, they
/// were asserting a form the service does not mint.
fn did_host(env: &AccessServiceAddress) -> String {
    host(env).replace(':', "%3A")
}

/// The base URL, without a trailing slash.
fn base(env: &AccessServiceAddress) -> String {
    env.access_service_url.trim_end_matches('/').to_string()
}

/// Resolve an address through the service, answering the status and body.
async fn lookup(
    env: &AccessServiceAddress,
    address: &str,
) -> anyhow::Result<(u16, serde_json::Value)> {
    let (local, domain) = address.rsplit_once('@').expect("an address with a domain");
    let response = reqwest::Client::new()
        .get(format!("{}/customer/{domain}/{local}/did.json", base(env)))
        .send()
        .await?;
    let status = response.status().as_u16();
    Ok((status, response.json().await?))
}

/// An activated customer resolves with `200` and a document naming the
/// key it enrolled with. This is the whole point of the endpoint: an
/// address in, that address's `did:key` out.
#[dialog_common::test]
async fn it_answers_an_activated_customer_with_their_key(
    env: AccessServiceAddress,
) -> anyhow::Result<()> {
    let customer = Ed25519Signer::generate().await?;
    env.activate_customer(&customer, "jsmith@example.com")
        .await?;

    let (status, document) = lookup(&env, "jsmith@example.com").await?;
    assert_eq!(status, 200, "an activated customer resolves");
    assert_eq!(
        document["id"],
        format!("did:web:{}:customer:example.com:jsmith", did_host(&env)),
        "the document names the DID that was resolved"
    );
    assert_eq!(
        document["alsoKnownAs"][0],
        customer.did().to_string(),
        "and carries the customer's did:key"
    );

    // The embedded key verifies against the same did:key, so a caller
    // that reads only the verification method reaches the same identity.
    let multibase = document["verificationMethod"][0]["publicKeyMultibase"]
        .as_str()
        .expect("a multikey verification method");
    assert_eq!(format!("did:key:{multibase}"), customer.did().to_string());
    Ok(())
}

/// The document says where the account syncs, not just who it is.
///
/// A second device holds an email and nothing else: resolving the
/// address has to yield both the account and its service, or the device
/// would need an endpoint from somewhere before it could ask for one.
#[dialog_common::test]
async fn it_names_the_service_the_account_syncs_with(
    env: AccessServiceAddress,
) -> anyhow::Result<()> {
    let customer = Ed25519Signer::generate().await?;
    env.activate_customer(&customer, "jsmith@example.com")
        .await?;

    let (_, document) = lookup(&env, "jsmith@example.com").await?;
    let service = &document["service"][0];
    assert_eq!(service["type"], "TonkAccessService");
    let endpoint = service["serviceEndpoint"]
        .as_str()
        .expect("the document carries a service endpoint");
    assert!(
        endpoint.ends_with("/ucan/"),
        "the endpoint is the service's UCAN address, got {endpoint}"
    );
    assert!(
        endpoint.contains(&host(&env)),
        "and names the host that answered, got {endpoint}"
    );
    Ok(())
}

/// An enrolled customer who has not clicked the activation link resolves
/// with `202`: the DID is real, but the address is claimed rather than
/// confirmed, and a caller about to act on it should be able to tell.
#[dialog_common::test]
async fn it_answers_an_unconfirmed_customer_with_accepted(
    env: AccessServiceAddress,
) -> anyhow::Result<()> {
    let customer = Ed25519Signer::generate().await?;
    env.enroll_customer(&customer, "pending@example.com")
        .await?;

    let (status, document) = lookup(&env, "pending@example.com").await?;
    assert_eq!(status, 202, "an unactivated customer is accepted, not ok");
    assert_eq!(document["status"], "Registered");
    assert_eq!(document["alsoKnownAs"][0], customer.did().to_string());
    Ok(())
}

/// Confirming the email moves the same address from `202` to `200`
/// without changing anything else about the answer. The status is the
/// only thing activation is allowed to move here.
#[dialog_common::test]
async fn it_moves_from_accepted_to_ok_on_confirmation(
    env: AccessServiceAddress,
) -> anyhow::Result<()> {
    let customer = Ed25519Signer::generate().await?;
    env.enroll_customer(&customer, "confirming@example.com")
        .await?;
    let (before, first) = lookup(&env, "confirming@example.com").await?;
    assert_eq!(before, 202);

    env.activate_customer(&customer, "confirming@example.com")
        .await?;
    let (after, second) = lookup(&env, "confirming@example.com").await?;
    assert_eq!(after, 200, "confirmation lifts the answer to ok");
    assert_eq!(second["status"], "Active");
    assert_eq!(
        first["alsoKnownAs"], second["alsoKnownAs"],
        "the key does not move when the status does"
    );
    Ok(())
}

/// An address nobody enrolled is a `404`. A caller learns the same thing
/// from an unregistered address as from one that never existed.
#[dialog_common::test]
async fn it_answers_an_unknown_address_with_not_found(
    env: AccessServiceAddress,
) -> anyhow::Result<()> {
    let (status, _) = lookup(&env, "nobody@example.com").await?;
    assert_eq!(status, 404);
    Ok(())
}

/// The address is normalized on both sides, so the casing a caller holds
/// does not have to match the casing the customer typed at enrollment.
#[dialog_common::test]
async fn it_resolves_an_address_whatever_its_casing(
    env: AccessServiceAddress,
) -> anyhow::Result<()> {
    let customer = Ed25519Signer::generate().await?;
    // Enrolled in mixed case, stored normalized.
    env.activate_customer(&customer, "JSmith@Example.COM")
        .await?;

    for spelling in [
        "jsmith@example.com",
        "JSmith@Example.COM",
        "JSMITH@EXAMPLE.COM",
    ] {
        let (status, document) = lookup(&env, spelling).await?;
        assert_eq!(status, 200, "{spelling} resolves");
        assert_eq!(
            document["alsoKnownAs"][0],
            customer.did().to_string(),
            "{spelling} reaches the same customer"
        );
    }
    Ok(())
}

/// A local part containing `+` survives the round trip. `did:web`
/// resolution percent-decodes each path segment, so `tag%2Balice` arrives
/// as `tag+alice`, and reading it as anything but a raw segment would
/// turn that `+` into a space and miss the row.
#[dialog_common::test]
async fn it_resolves_a_local_part_that_needs_encoding(
    env: AccessServiceAddress,
) -> anyhow::Result<()> {
    let customer = Ed25519Signer::generate().await?;
    env.activate_customer(&customer, "tag+alice@web.mail")
        .await?;

    let (status, document) = lookup(&env, "tag+alice@web.mail").await?;
    assert_eq!(status, 200, "a + in the local part resolves");
    assert_eq!(document["alsoKnownAs"][0], customer.did().to_string());
    assert_eq!(
        document["id"],
        format!("did:web:{}:customer:web.mail:tag%2Balice", did_host(&env)),
        "and the DID it names carries the encoded form"
    );
    Ok(())
}

/// The two-segment lookup does not shadow the single-segment registration
/// probe. Both live under `/customer/`, and the probe reads its whole
/// remainder as a DID, so matching it first would swallow every lookup.
#[dialog_common::test]
async fn it_leaves_the_registration_probe_reachable(
    env: AccessServiceAddress,
) -> anyhow::Result<()> {
    let customer = Ed25519Signer::generate().await?;
    env.activate_customer(&customer, "probe@example.com")
        .await?;

    let receipt: serde_json::Value = reqwest::Client::new()
        .get(format!("{}/customer/{}", base(&env), customer.did()))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(receipt["status"], "Active", "the probe still answers");

    let (status, _) = lookup(&env, "probe@example.com").await?;
    assert_eq!(status, 200, "and the lookup answers too");
    Ok(())
}

/// The lookup is called from the browser by the invite flow, so it
/// answers with the same permissive CORS header every other public route
/// on this service does.
#[dialog_common::test]
async fn it_answers_with_cors_for_the_browser(env: AccessServiceAddress) -> anyhow::Result<()> {
    let customer = Ed25519Signer::generate().await?;
    env.activate_customer(&customer, "cors@example.com").await?;

    let response = reqwest::Client::new()
        .get(format!("{}/customer/example.com/cors/did.json", base(&env)))
        .send()
        .await?;
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .and_then(|value| value.to_str().ok()),
        Some("*")
    );
    Ok(())
}

/// A DID a caller built from an address finds the customer that address
/// belongs to. This closes the loop the endpoint exists for: the half
/// that mints the DID and the half that resolves it agree on the form.
#[dialog_common::test]
async fn it_resolves_a_did_built_by_a_caller(env: AccessServiceAddress) -> anyhow::Result<()> {
    let customer = Ed25519Signer::generate().await?;
    env.activate_customer(&customer, "round.trip@example.com")
        .await?;

    let did = customer_did(&host(&env), "round.trip@example.com").expect("a resolvable address");
    // Resolution turns the method-specific id into a path and appends
    // did.json, which is the request this service answers.
    let path = did
        .strip_prefix(&format!("did:web:{}:", did_host(&env)))
        .expect("the DID sits under this host")
        .replace(':', "/");
    let document: serde_json::Value = reqwest::Client::new()
        .get(format!("{}/{path}/did.json", base(&env)))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(document["alsoKnownAs"][0], customer.did().to_string());
    let _: Did = document["alsoKnownAs"][0]
        .as_str()
        .expect("a did:key string")
        .parse()
        .map_err(|error| anyhow::anyhow!("the answered key does not parse: {error:?}"))?;
    Ok(())
}

/// Only a settled answer is cacheable. A `202` says the customer has not
/// confirmed yet and a `404` that nobody has claimed the address; both
/// are about to change, and an invite flow polling an address must not be
/// told for a minute that the person it just invited is still unknown.
#[dialog_common::test]
async fn it_lets_only_a_settled_answer_be_cached(env: AccessServiceAddress) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let cache_control = |response: &reqwest::Response| {
        response
            .headers()
            .get("cache-control")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string()
    };

    // Nobody has claimed this address yet.
    let missing = client
        .get(format!(
            "{}/customer/example.com/uncached/did.json",
            base(&env)
        ))
        .send()
        .await?;
    assert_eq!(missing.status().as_u16(), 404);
    assert_eq!(
        cache_control(&missing),
        "no-store",
        "a 404 is not cacheable"
    );

    // Enrolled but unconfirmed.
    let customer = Ed25519Signer::generate().await?;
    env.enroll_customer(&customer, "uncached@example.com")
        .await?;
    let pending = client
        .get(format!(
            "{}/customer/example.com/uncached/did.json",
            base(&env)
        ))
        .send()
        .await?;
    assert_eq!(pending.status().as_u16(), 202);
    assert_eq!(
        cache_control(&pending),
        "no-store",
        "a 202 is not cacheable"
    );

    // Confirmed, and now settled enough to cache.
    env.activate_customer(&customer, "uncached@example.com")
        .await?;
    let settled = client
        .get(format!(
            "{}/customer/example.com/uncached/did.json",
            base(&env)
        ))
        .send()
        .await?;
    assert_eq!(settled.status().as_u16(), 200);
    assert_eq!(
        cache_control(&settled),
        "public, max-age=60",
        "a settled answer is cacheable, briefly"
    );
    Ok(())
}
