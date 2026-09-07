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

/// Every hand-built claim must declare each field it sets.
///
/// This is the invariant the worker enforces server-side: a parameter the
/// inlined concept does not declare is rejected outright —
/// `invalid claim: field "…" is not declared by this concept` — so the
/// whole transact fails with a 400 and the interaction silently does
/// nothing.
///
/// The per-command checks above cannot see it. They assert that the
/// claim's JSON *text* contains each attribute the handler triggers on,
/// which the descriptor half alone satisfies; a `parameters` map that has
/// drifted from that descriptor passes every one of them. Renaming a space
/// shipped broken exactly that way: the descriptor was migrated to
/// `{name, space}` while the parameters kept sending `value` and a
/// `rename-repository` marker from the pre-namespace shape.
#[dialog_common::test]
fn every_hand_built_claim_declares_the_fields_it_sets() {
    use tonk_fab::logic::Dock;

    let space = "did:key:z6Mk";
    let claims = [
        ("dock", logic::dock_claim_json(Dock::TopLeft)),
        ("collapsed", logic::collapsed_claim_json(true)),
        (
            "promote",
            logic::promote_claim_json(space, "did:key:z6Mm", "chain"),
        ),
        ("pause", logic::pause_claim_json(space, 1.0)),
        (
            "rename-repo",
            logic::rename_repo_claim_json(space, "Renamed"),
        ),
        (
            "rename-repo (blank)",
            logic::rename_repo_claim_json(space, ""),
        ),
        ("profile-rename", logic::profile_rename_claim_json("Ada")),
        ("invite", logic::invite_claim_json(space, 1.0)),
        (
            "enable-sync",
            logic::enable_sync_claim_json(space, "https://example.test/ucan/", true, 1.0),
        ),
        // The remote and share halves are conditional on both sides, so the
        // bare form is checked too — dropping one from `with` but not from
        // `parameters` is the same defect in a branch the full form hides.
        (
            "enable-sync (bare)",
            logic::enable_sync_claim_json(space, "", false, 1.0),
        ),
    ];

    for (label, claim) in claims {
        let application = &claim["claims"][0]["application"];
        let declared = application["predicate"]["concept"]["with"]
            .as_object()
            .unwrap_or_else(|| panic!("{label}: the claim must inline a `with` map"));
        let parameters = application["parameters"]
            .as_object()
            .unwrap_or_else(|| panic!("{label}: the claim must carry parameters"));

        assert!(
            !parameters.is_empty(),
            "{label}: a claim that sets nothing would assert nothing",
        );
        for field in parameters.keys() {
            // `this` is the one parameter that is not a concept field: the
            // worker mints it from `(descriptor, parameters)` when absent,
            // and pins it when present.
            if field == "this" {
                continue;
            }
            assert!(
                declared.contains_key(field),
                "{label}: parameter `{field}` is not declared by the inlined \
                 concept ({:?}) — the worker rejects this claim with a 400",
                declared.keys().collect::<Vec<_>>(),
            );
        }
    }
}
