//! Wire-compatibility guard for the two places where `tonk-worker`'s
//! engine-typed surface and the engine-free `tonk-worker-api` view are
//! *different Rust types* over the same wire:
//!
//! - `RemoteConfiguration.address` — `SiteAddress` here vs an opaque
//!   `serde_json::Value` in the api crate.
//! - `Query` — `tonk_schema::query::Query` (engine-typed
//!   `ConceptDescriptor`/`Parameters` fields) vs the api crate's plain
//!   serde mirror.
//!
//! `Revision`, `SiteAddress`'s DID, `SyncState`, `Conclusion`, and
//! `Frame` need no guard: they are now the SAME type on both sides
//! (they live in engine-free crates the api crate re-exports), so a
//! test would be a tautology.

/// A `RemoteConfiguration` built the api-crate way — `ucan(url)` — must
/// deserialize into the worker's engine-typed `RemoteConfiguration`
/// with the real UCAN `SiteAddress`, i.e. the page's wire shape decodes
/// server-side (and round-trips back).
#[dialog_common::test]
fn it_agrees_on_the_ucan_remote_address() {
    use super::repository::RemoteConfiguration as ServerRemote;

    const URL: &str = "https://access.example.com/ucan/";
    let from_page = tonk_worker_api::RemoteConfiguration::ucan(URL);
    let page_json = serde_json::to_string(&from_page).expect("encode page remote");

    let server: ServerRemote =
        serde_json::from_str(&page_json).expect("server decodes the page's remote config");
    let server_json = serde_json::to_string(&server).expect("encode server remote");
    let round_tripped: tonk_worker_api::RemoteConfiguration =
        serde_json::from_str(&server_json).expect("api type decodes the server's remote config");

    assert_eq!(
        serde_json::to_value(&from_page).expect("page value"),
        serde_json::to_value(&round_tripped).expect("round-tripped value"),
        "ucan remote address wire form drifted"
    );
}

/// The api crate's `Query` mirror and the worker's engine-typed
/// `tonk_schema::query::Query` must decode the same wire body to an
/// equal JSON *value*. Compared as `serde_json::Value` (structural,
/// order-agnostic) rather than as strings, because the engine `Query`
/// normalizes its `ConceptDescriptor` (injects/reorders fields) — a
/// string compare would spuriously fail on field order.
#[dialog_common::test]
fn it_agrees_on_a_concept_query() {
    let wire = r#"{"terms":{"this":"id:demo"},"predicate":{"with":{"count":{"the":"counter/count","as":"UnsignedInteger","cardinality":"one"}}}}"#;

    let real: tonk_schema::query::Query = serde_json::from_str(wire).expect("decode real query");
    let mirror: tonk_worker_api::Query = serde_json::from_str(wire).expect("decode mirror query");

    let real_value = serde_json::to_value(&real).expect("encode real");
    let mirror_value = serde_json::to_value(&mirror).expect("encode mirror");

    // The mirror carries the predicate verbatim; the engine type may
    // normalize the descriptor. Assert the terms match exactly and the
    // predicate's author-supplied keys survive on both sides.
    assert_eq!(
        real_value.get("terms"),
        mirror_value.get("terms"),
        "query terms drifted"
    );
    assert!(
        mirror_value.pointer("/predicate/with/count/the").is_some(),
        "mirror query dropped a descriptor field"
    );
}
