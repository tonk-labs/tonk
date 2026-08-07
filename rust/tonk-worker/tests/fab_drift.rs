//! The FAB must keep working against a space branch seeded by ANY past
//! `core.yaml`.
//!
//! `core.yaml` is seeded once at repo creation and never re-seeded, so every
//! existing space's descriptors are frozen at the version that created it.
//! The FAB survives that by consulting nothing seeded: it reads raw attribute
//! URIs and inlines its own command descriptors. That is why there is no
//! old-library fixture here — there is nothing seeded for it to be checked
//! against.
//!
//! The load-bearing invariant is now nominal: every hand-built request names
//! the exact registered command kind and uses the semantic argument names its
//! authoritative branch schema validates.
//!
//! Native-only, mirroring `standard_library.rs`: no filesystem on wasm, and
//! this needs no running system.

#![cfg(not(target_arch = "wasm32"))]

use tonk_fab::logic;

#[dialog_common::test]
fn it_builds_a_nominal_rename_invocation() {
    let claim = logic::rename_repo_claim_json("did:key:z6Mk", "Renamed");
    let invocation = &claim["claims"][0];
    assert_eq!(invocation["op"], "invoke");
    assert_eq!(invocation["command"], "tonk:rename-repository");
    assert_eq!(invocation["arguments"]["space"], "did:key:z6Mk");
    assert_eq!(invocation["arguments"]["name"], "Renamed");
}

#[dialog_common::test]
fn it_builds_a_nominal_invite_invocation() {
    let claim = logic::invite_claim_json("did:key:z6Mk", 1.0);
    let invocation = &claim["claims"][0];
    assert_eq!(invocation["op"], "invoke");
    assert_eq!(invocation["command"], "tonk:invite");
    assert_eq!(invocation["arguments"]["space"], "did:key:z6Mk");
    assert_eq!(invocation["arguments"]["time"], 1.0);
}

#[dialog_common::test]
fn it_builds_a_nominal_create_space_invocation() {
    let claim = logic::create_space_claim_json("Untitled", "https://x", "https://x/rev", "wiki");
    let invocation = &claim["claims"][0];
    assert_eq!(invocation["op"], "invoke");
    assert_eq!(invocation["command"], "id:space/create");
    assert_eq!(invocation["arguments"]["name"], "Untitled");
    assert_eq!(invocation["arguments"]["remote"], "https://x");
    assert_eq!(invocation["arguments"]["revocation"], "https://x/rev");
    assert_eq!(invocation["arguments"]["template"], "wiki");
}

#[dialog_common::test]
fn it_builds_a_nominal_pause_invocation() {
    let claim = logic::pause_claim_json("tonk:pause-sync", "did:key:z6Mk", 1.0);
    let invocation = &claim["claims"][0];
    assert_eq!(invocation["op"], "invoke");
    assert_eq!(invocation["command"], "tonk:pause-sync");
    assert_eq!(invocation["arguments"]["space"], "did:key:z6Mk");
}

#[dialog_common::test]
fn it_reads_the_repo_name_attribute_the_schema_writes() {
    // The FAB reads a raw attribute; the worker writes facts under the
    // schema's domain type. If the two diverge the chip silently blanks.
    let body = logic::repo_name_query_body("did:key:z6Mk").expect("builds");
    assert!(body.contains("xyz.tonk.repo/name"));
}
