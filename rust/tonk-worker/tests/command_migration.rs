//! Every migrated command must still decode the shape a branch seeded
//! before the migration asserts.
//!
//! A command is matched structurally: the set of attribute names a
//! transient carries is its whole identity. Each command here changed
//! that set — its fields moved out of the DOM read paths that used to
//! fill them and into the command's own `xyz.tonk.command.<verb>`
//! namespace, and the marker fields that existed only to stop two
//! commands decoding as each other went away with them.
//!
//! `core.yaml` is seeded once at repo creation and never re-seeded, so
//! every existing space still holds the old descriptors and still posts
//! the old attributes. If those stopped decoding, the transient would
//! commit, no handler would run, and the UI would look successful — the
//! exact silent failure this suite exists to prevent.
//!
//! What makes that survivable without the old shape becoming permanent
//! is [`dialog_reactor::Migrated`]: the handler is written against the
//! current shape alone, and the legacy one reaches it through a `From`
//! impl. Retiring the old shape is deleting `tonk_schema::command::legacy`,
//! those `From` impls, and one type parameter per handler.
//!
//! Native-only, mirroring `standard_library.rs`: this needs no running
//! system.

#![cfg(not(target_arch = "wasm32"))]

use dialog_artifacts::{Artifact, Entity};
use dialog_reactor::{EntityFacts, Migrated};
use tonk_schema::command;

fn entity(source: &str) -> Entity {
    source.parse().expect("entity URI")
}

/// One transient entity's facts, from `(attribute, value)` pairs.
///
/// Built as raw artifacts rather than through a typed concept: the whole
/// point is to post attribute names no current type declares.
fn facts(pairs: Vec<(&str, Value)>) -> EntityFacts {
    let this = entity("did:key:zCommand");
    pairs
        .into_iter()
        .map(|(attribute, value)| Artifact {
            the: attribute.parse().expect("attribute name"),
            of: this.clone(),
            is: match value {
                Value::Text(source) => dialog_artifacts::Value::String(source),
                Value::Entity(source) => dialog_artifacts::Value::Entity(entity(&source)),
                Value::Float(number) => dialog_artifacts::Value::Float(number),
            },
            cause: None,
        })
        .collect()
}

enum Value {
    Text(String),
    Entity(String),
    Float(f64),
}

fn text(value: &str) -> Value {
    Value::Text(value.to_string())
}

fn uri(value: &str) -> Value {
    Value::Entity(value.to_string())
}

#[dialog_common::test]
fn a_legacy_create_space_still_decodes() {
    let command: Migrated<command::CreateSpace, command::legacy::CreateSpace> = Migrated::new();
    let decoded = command
        .decode(&facts(vec![(
            "dom.event.current-target.elements.name/value",
            text("Untitled"),
        )]))
        .expect("a name-only transient from an older profile descriptor");
    assert_eq!(decoded.name.0, "Untitled");
}

#[dialog_common::test]
fn a_current_create_space_decodes() {
    let command: Migrated<command::CreateSpace, command::legacy::CreateSpace> = Migrated::new();
    let decoded = command
        .decode(&facts(vec![(
            "xyz.tonk.command.create-space/name",
            text("Untitled"),
        )]))
        .expect("the shape the app posts now");
    assert_eq!(decoded.name.0, "Untitled");
}

#[dialog_common::test]
fn a_legacy_remove_space_still_decodes() {
    let command: Migrated<command::RemoveSpace, command::legacy::RemoveSpace> = Migrated::new();
    let decoded = command
        .decode(&facts(vec![(
            "dom.event.current-target.dataset/remove",
            uri("did:key:zSpace"),
        )]))
        .expect("the Hub's confirm form on an older profile branch");
    assert_eq!(decoded.subject.0.to_string(), "did:key:zSpace");
}

#[dialog_common::test]
fn a_legacy_invite_still_decodes_without_its_marker_meaning_anything() {
    let command: Migrated<command::Invite, command::legacy::Invite> = Migrated::new();
    let decoded = command
        .decode(&facts(vec![
            ("dom.event/time-stamp", Value::Float(17.0)),
            (
                "dom.event.current-target.dataset/invite",
                uri("tonk:invite"),
            ),
        ]))
        .expect("a share click on an older space branch");
    assert_eq!(decoded.time.0, 17.0);
}

#[dialog_common::test]
fn a_legacy_pause_sync_still_decodes() {
    let command: Migrated<command::PauseSync, command::legacy::PauseSync> = Migrated::new();
    let decoded = command
        .decode(&facts(vec![
            ("dom.event/time-stamp", Value::Float(3.0)),
            ("xyz.tonk.pause-sync/space", uri("did:key:zSpace")),
            (
                "dom.event.current-target.dataset/pause-sync",
                uri("tonk:pause-sync"),
            ),
        ]))
        .expect("an alt-click dispatched by an older FAB bundle");
    assert_eq!(decoded.space.0.to_string(), "did:key:zSpace");
    assert_eq!(decoded.time.0, 3.0);
}

#[dialog_common::test]
fn a_legacy_profile_rename_still_decodes() {
    let command: Migrated<command::ProfileRename, command::legacy::ProfileRename> = Migrated::new();
    let decoded = command
        .decode(&facts(vec![
            ("dom.event.current-target/value", text("Ada")),
            (
                "dom.event.current-target.dataset/rename",
                uri("tonk:profile"),
            ),
        ]))
        .expect("the identity chip on an older profile branch");
    assert_eq!(decoded.name.0, "Ada");
}

#[dialog_common::test]
fn a_legacy_rename_repository_still_decodes() {
    let command: Migrated<command::RenameRepository, command::legacy::RenameRepository> =
        Migrated::new();
    let decoded = command
        .decode(&facts(vec![
            ("dom.event.current-target/value", text("Pictures")),
            ("xyz.tonk.rename-repository/space", uri("did:key:zSpace")),
            (
                "dom.event.current-target.dataset/rename-repository",
                uri("tonk:rename-repository"),
            ),
        ]))
        .expect("the FAB's name chip on an older bundle");
    assert_eq!(decoded.name.0, "Pictures");
    assert_eq!(decoded.space.0.to_string(), "did:key:zSpace");
}

#[dialog_common::test]
fn a_legacy_expel_member_still_decodes() {
    let command: Migrated<command::ExpelMember, command::legacy::ExpelMember> = Migrated::new();
    let decoded = command
        .decode(&facts(vec![(
            "dom.event.current-target.dataset/expel",
            uri("did:key:zMember"),
        )]))
        .expect("a roster row on an older space branch");
    assert_eq!(decoded.member.0.to_string(), "did:key:zMember");
}

#[dialog_common::test]
fn a_legacy_join_still_decodes() {
    let command: Migrated<command::Join, command::legacy::Join> = Migrated::new();
    let decoded = command
        .decode(&facts(vec![(
            "dom.event.detail/href",
            text("https://tonk.test/join#seed"),
        )]))
        .expect("the /join page on an older profile branch");
    assert_eq!(decoded.url.0, "https://tonk.test/join#seed");
}

/// The pair the marker fields existed for.
///
/// `Invite` and `PauseSync` were both `{this, time}` reading one shared
/// `dom.event/time-stamp`, so each carried an extra attribute purely to
/// stay distinct. In the current shapes each `time` is in its own
/// namespace, and this is what says so: neither command decodes the
/// other's transient, with not a marker in sight.
#[dialog_common::test]
fn an_invite_and_a_pause_no_longer_need_a_marker_to_stay_apart() {
    let invite: Migrated<command::Invite, command::legacy::Invite> = Migrated::new();
    let pause: Migrated<command::PauseSync, command::legacy::PauseSync> = Migrated::new();

    let an_invite = facts(vec![("xyz.tonk.command.invite/time", Value::Float(1.0))]);
    let a_pause = facts(vec![
        ("xyz.tonk.command.pause-sync/time", Value::Float(1.0)),
        ("xyz.tonk.pause-sync/space", uri("did:key:zSpace")),
    ]);

    assert!(invite.matches(&an_invite));
    assert!(pause.matches(&a_pause));
    assert!(!pause.matches(&an_invite), "an invite is not a pause");
    assert!(!invite.matches(&a_pause), "a pause is not an invite");
}

/// The other pair: a rename of the space and a rename of the person both
/// read `currentTarget.value`, and before the marker every space rename
/// also renamed the profile.
#[dialog_common::test]
fn the_two_renames_no_longer_need_a_marker_to_stay_apart() {
    let repository: Migrated<command::RenameRepository, command::legacy::RenameRepository> =
        Migrated::new();
    let profile: Migrated<command::ProfileRename, command::legacy::ProfileRename> = Migrated::new();

    let a_repository_rename = facts(vec![
        ("xyz.tonk.command.rename-repository/name", text("Pictures")),
        ("xyz.tonk.rename-repository/space", uri("did:key:zSpace")),
    ]);
    let a_profile_rename = facts(vec![("xyz.tonk.command.profile-rename/name", text("Ada"))]);

    assert!(repository.matches(&a_repository_rename));
    assert!(profile.matches(&a_profile_rename));
    assert!(
        !profile.matches(&a_repository_rename),
        "renaming a space must not rename the person",
    );
    assert!(!repository.matches(&a_profile_rename));
}

/// And the pair that made a passkey prompt appear mid-keystroke.
#[dialog_common::test]
fn a_lookup_no_longer_decodes_as_a_registration() {
    let lookup: Migrated<command::CheckEmail, command::legacy::CheckEmail> = Migrated::new();
    let registration: Migrated<command::RegisterAccount, command::legacy::RegisterAccount> =
        Migrated::new();

    let asking = facts(vec![(
        "xyz.tonk.command.check-email/email",
        text("ada@example.com"),
    )]);
    let registering = facts(vec![(
        "xyz.tonk.command.register-account/email",
        text("ada@example.com"),
    )]);

    assert!(lookup.matches(&asking));
    assert!(registration.matches(&registering));
    assert!(
        !registration.matches(&asking),
        "asking whether an address is free must not create the account",
    );
    assert!(!lookup.matches(&registering));
}
