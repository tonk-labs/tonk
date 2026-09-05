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
//! The load-bearing invariant is that a hand-built claim carries exactly the
//! attributes its handler indexes on. If they drift apart the command decodes
//! as nothing, the handler never runs, and the UI still looks successful —
//! the precise failure this design exists to prevent.
//!
//! Native-only, mirroring `standard_library.rs`: no filesystem on wasm, and
//! this needs no running system.

#![cfg(not(target_arch = "wasm32"))]

use tonk_fab::logic;

#[dialog_common::test]
fn it_builds_rename_claims_carrying_every_attribute_the_handler_triggers_on() {
    use dialog_reactor::Decode as _;

    let triggers = tonk_schema::command::RenameRepository::trigger_attributes();
    assert!(
        !triggers.is_empty(),
        "the command must declare trigger attributes"
    );

    let claim = logic::rename_repo_claim_json("did:key:z6Mk", "Renamed").to_string();
    for attribute in &triggers {
        assert!(
            claim.contains(attribute.as_str()),
            "hand-built rename claim must carry trigger attribute {attribute}"
        );
    }
}

#[dialog_common::test]
fn it_builds_invite_claims_carrying_every_attribute_the_handler_triggers_on() {
    use dialog_reactor::Decode as _;

    let triggers = tonk_schema::command::Invite::trigger_attributes();
    assert!(
        !triggers.is_empty(),
        "the command must declare trigger attributes"
    );

    let claim = logic::invite_claim_json("did:key:z6Mk", 1.0).to_string();
    for attribute in &triggers {
        assert!(
            claim.contains(attribute.as_str()),
            "hand-built invite claim must carry trigger attribute {attribute}"
        );
    }
}

#[dialog_common::test]
fn it_builds_create_space_claims_carrying_every_attribute_the_handler_triggers_on() {
    use dialog_reactor::Decode as _;

    // Deliberately name-only: see `it_decodes_create_space_from_name_only_facts`
    // in `dialog-reactor/src/command.rs`, which pins that a frozen, older
    // profile descriptor (name field alone) must still decode.
    let triggers = tonk_schema::command::CreateSpace::trigger_attributes();
    assert!(
        !triggers.is_empty(),
        "the command must declare trigger attributes"
    );

    let claim = logic::create_space_claim_json("Untitled").to_string();
    for attribute in &triggers {
        assert!(
            claim.contains(attribute.as_str()),
            "hand-built create-space claim must carry trigger attribute {attribute}"
        );
    }
}

#[dialog_common::test]
fn it_builds_pause_claims_carrying_every_attribute_the_handler_triggers_on() {
    use dialog_reactor::Decode as _;

    let triggers = tonk_schema::command::PauseSync::trigger_attributes();
    assert!(
        !triggers.is_empty(),
        "the command must declare trigger attributes"
    );

    let claim = logic::pause_claim_json("did:key:z6Mk", 1.0).to_string();
    for attribute in &triggers {
        assert!(
            claim.contains(attribute.as_str()),
            "hand-built pause claim must carry trigger attribute {attribute}"
        );
    }
}

#[dialog_common::test]
fn it_reads_the_repo_name_attribute_the_schema_writes() {
    // The FAB reads a raw attribute; the worker writes facts under the
    // schema's domain type. If the two diverge the chip silently blanks.
    let body = logic::repo_name_query_body("did:key:z6Mk").expect("builds");
    assert!(body.contains("xyz.tonk.repo/name"));
}
