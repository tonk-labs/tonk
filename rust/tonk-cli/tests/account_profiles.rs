use std::path::Path;

use anyhow::Result;
use serde_json::json;
use tonk_cli::account_profiles::{LEGACY_PROFILE_ID, NativeProfileId, NativeProfileStore};
use tonk_cli::spot::{Registry, SpotEntry};

fn write_json(path: &Path, value: serde_json::Value) -> Result<()> {
    std::fs::write(path, serde_json::to_vec_pretty(&value)?)?;
    Ok(())
}

#[test]
fn it_bootstraps_the_legacy_profile_without_moving_state() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let root = tmp.path().join("state");
    let work = tmp.path().join("work");
    let site = root.join("spots/garden");
    std::fs::create_dir_all(&work)?;
    std::fs::create_dir_all(&site)?;
    std::fs::create_dir_all(root.join("account"))?;
    std::fs::write(site.join("sentinel"), b"site bytes")?;
    std::fs::write(root.join("account/sentinel"), b"account bytes")?;
    let work_key = work.canonicalize()?.display().to_string();
    write_json(
        &root.join("spots.json"),
        json!({
            "spots": { "garden": { "site": site.canonicalize()? } },
            "bindings": { (work_key): "garden" },
            "futureSpotField": { "preserved": true }
        }),
    )?;
    let spots_before = std::fs::read(root.join("spots.json"))?;

    let profiles = NativeProfileStore::at(&root);
    let registry = profiles.load_or_bootstrap()?;

    assert_eq!(
        registry.selected.as_ref().map(NativeProfileId::as_str),
        Some(LEGACY_PROFILE_ID)
    );
    assert_eq!(registry.profiles.len(), 1);
    let legacy = &registry.profiles[&NativeProfileId::legacy()];
    assert_eq!(legacy.label, "default");
    assert_eq!(legacy.dialog_profile_name, "tonk");
    let bound = &registry.bindings[&work.canonicalize()?];
    assert_eq!(bound.profile, NativeProfileId::legacy());
    assert_eq!(bound.space, "garden");

    assert_eq!(std::fs::read(root.join("spots.json"))?, spots_before);
    assert_eq!(std::fs::read(site.join("sentinel"))?, b"site bytes");
    assert_eq!(
        std::fs::read(root.join("account/sentinel"))?,
        b"account bytes"
    );
    Ok(())
}

#[test]
fn it_starts_empty_without_creating_a_dialog_profile() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let root = tmp.path().join("state");
    let profiles = NativeProfileStore::at(&root);

    let registry = profiles.load_or_bootstrap()?;

    assert!(registry.selected.is_none());
    assert!(registry.profiles.is_empty());
    assert!(!root.join("profiles.json").exists());
    assert!(
        !root.exists(),
        "read-only bootstrap must not create install state"
    );
    Ok(())
}

#[test]
fn it_rejects_unknown_versions_corrupt_json_and_dangling_bindings() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let root = tmp.path().join("state");
    std::fs::create_dir_all(&root)?;
    let profiles = NativeProfileStore::at(&root);

    std::fs::write(root.join("profiles.json"), b"{not json")?;
    let error = profiles.load_or_bootstrap().expect_err("corrupt registry");
    assert!(error.to_string().contains("profiles.json"), "{error}");

    write_json(&root.join("profiles.json"), json!({ "version": 2 }))?;
    let error = profiles.load_or_bootstrap().expect_err("unknown version");
    assert!(
        error.to_string().contains("unsupported version 2"),
        "{error}"
    );

    write_json(
        &root.join("profiles.json"),
        json!({
            "version": 1,
            "selected": "legacy",
            "profiles": {
                "legacy": {
                    "label": "default",
                    "dialogProfileName": "tonk",
                    "accountRoot": null,
                    "ceremonyOrigin": null,
                    "defaultAccessRemote": null,
                    "defaultRevocationRelay": null
                }
            },
            "bindings": { (tmp.path().display().to_string()): { "profile": "missing", "space": "garden" } }
        }),
    )?;
    let error = profiles
        .load_or_bootstrap()
        .expect_err("unknown binding profile");
    assert!(error.to_string().contains("missing"), "{error}");

    write_json(
        &root.join("profiles.json"),
        json!({
            "version": 1,
            "selected": "legacy",
            "profiles": {
                "legacy": {
                    "label": "default",
                    "dialogProfileName": "tonk",
                    "accountRoot": null,
                    "ceremonyOrigin": null,
                    "defaultAccessRemote": null,
                    "defaultRevocationRelay": null
                }
            },
            "bindings": { (tmp.path().display().to_string()): { "profile": "legacy", "space": "garden" } }
        }),
    )?;
    let error = profiles
        .load_or_bootstrap()
        .expect_err("missing local space");
    assert!(error.to_string().contains("garden"), "{error}");
    Ok(())
}

#[test]
fn it_preserves_unknown_registry_and_profile_fields() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let root = tmp.path().join("state");
    std::fs::create_dir_all(&root)?;
    write_json(
        &root.join("profiles.json"),
        json!({
            "version": 1,
            "selected": "legacy",
            "profiles": {
                "legacy": {
                    "label": "default",
                    "dialogProfileName": "tonk",
                    "accountRoot": null,
                    "ceremonyOrigin": null,
                    "defaultAccessRemote": null,
                    "defaultRevocationRelay": null,
                    "futureProfileField": { "kept": 1 }
                }
            },
            "bindings": {},
            "futureRegistryField": ["kept"]
        }),
    )?;
    let profiles = NativeProfileStore::at(&root);

    let registry = profiles.load_or_bootstrap()?;
    profiles.save(&registry)?;

    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("profiles.json"))?)?;
    assert_eq!(saved["futureRegistryField"], json!(["kept"]));
    assert_eq!(
        saved["profiles"]["legacy"]["futureProfileField"],
        json!({ "kept": 1 })
    );
    Ok(())
}

#[test]
fn it_creates_distinct_pending_profiles_with_isolated_state_roots() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let root = tmp.path().join("state");
    let profiles = NativeProfileStore::at(&root);

    let a = profiles.create_pending_with_bytes(Some("personal"), [0x11; 16])?;
    let b = profiles.create_pending_with_bytes(Some("work"), [0x22; 16])?;

    assert_eq!(a.id.as_str(), "p-11111111111111111111111111111111");
    assert_eq!(
        a.record.dialog_profile_name,
        "tonk-11111111111111111111111111111111"
    );
    assert_eq!(b.id.as_str(), "p-22222222222222222222222222222222");
    assert_eq!(
        b.record.dialog_profile_name,
        "tonk-22222222222222222222222222222222"
    );
    assert_ne!(a.store.root(), b.store.root());
    assert_ne!(a.store.root(), root.as_path());
    assert_ne!(b.store.root(), root.as_path());
    assert_eq!(a.site_config().profile_name, a.record.dialog_profile_name);
    assert_eq!(b.site_config().profile_name, b.record.dialog_profile_name);
    assert_eq!(profiles.load_or_bootstrap()?.selected, Some(a.id));
    Ok(())
}

fn register_space(
    context: &tonk_cli::account_profiles::NativeProfileContext,
    name: &str,
) -> Result<()> {
    let site = context.store.canonical_site(name);
    std::fs::create_dir_all(&site)?;
    let site = site.canonicalize()?;
    let mut registry = Registry::default();
    registry.spots.insert(name.to_owned(), SpotEntry { site });
    context.store.save(&registry)?;
    Ok(())
}

#[test]
fn it_resolves_a_directory_binding_to_its_profile_even_when_another_profile_is_selected()
-> Result<()> {
    let tmp = tempfile::tempdir()?;
    let root = tmp.path().join("state");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work)?;
    let profiles = NativeProfileStore::at(&root);
    let a = profiles.create_pending_with_bytes(Some("personal"), [0x33; 16])?;
    let b = profiles.create_pending_with_bytes(Some("work"), [0x44; 16])?;
    register_space(&a, "garden")?;
    register_space(&b, "garden")?;
    profiles.bind(&a.id, "garden", &work)?;
    profiles.select("work")?;

    let bound = profiles.resolve(None, None, Some(&work))?;
    assert_eq!(bound.profile.id, a.id);
    assert_eq!(bound.name, "garden");

    let flagged = profiles.resolve(Some("garden"), None, Some(&work))?;
    assert_eq!(flagged.profile.id, b.id);

    let from_env = profiles.resolve(None, Some("garden"), Some(&work))?;
    assert_eq!(from_env.profile.id, b.id);
    Ok(())
}

#[test]
fn it_keeps_equal_names_and_canonical_paths_disjoint_between_profiles() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let profiles = NativeProfileStore::at(tmp.path().join("state"));
    let a = profiles.create_pending_with_bytes(Some("personal"), [0x55; 16])?;
    let b = profiles.create_pending_with_bytes(Some("work"), [0x66; 16])?;
    register_space(&a, "garden")?;
    register_space(&b, "garden")?;

    assert_ne!(a.store.registry_path(), b.store.registry_path());
    assert_ne!(
        a.store.canonical_site("garden"),
        b.store.canonical_site("garden")
    );
    assert_ne!(a.store.account_dir(), b.store.account_dir());
    assert_ne!(a.record.dialog_profile_name, b.record.dialog_profile_name);
    Ok(())
}

#[test]
fn it_refuses_to_bind_a_space_absent_from_the_selected_profile() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let profiles = NativeProfileStore::at(tmp.path().join("state"));
    let a = profiles.create_pending_with_bytes(Some("personal"), [0x77; 16])?;
    let b = profiles.create_pending_with_bytes(Some("work"), [0x88; 16])?;
    register_space(&a, "personal-only")?;
    register_space(&b, "work-only")?;
    profiles.select("work")?;

    let error = profiles
        .bind(&b.id, "personal-only", tmp.path())
        .expect_err("the selected profile cannot borrow another profile's names");
    let message = error.to_string();
    assert!(message.contains("work-only"), "{message}");
    assert!(!message.contains("registered: personal-only"), "{message}");
    Ok(())
}

#[test]
fn it_validates_labels_and_selects_by_label_or_exact_id() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let profiles = NativeProfileStore::at(tmp.path().join("state"));
    let first = profiles.create_pending_with_bytes(None, [0x91; 16])?;
    let second = profiles.create_pending_with_bytes(None, [0x92; 16])?;
    assert_eq!(first.record.label, "account");
    assert_eq!(second.record.label, "account-2");

    let duplicate = profiles
        .create_pending_with_bytes(Some("ACCOUNT"), [0x93; 16])
        .expect_err("labels are unique without regard to case");
    assert!(
        duplicate
            .to_string()
            .contains("invalid account profile label")
    );
    assert!(
        profiles
            .create_pending_with_bytes(Some("bad label"), [0x94; 16])
            .is_err()
    );

    assert_eq!(profiles.select("account-2")?.id, second.id);
    assert_eq!(profiles.select(first.id.as_str())?.id, first.id);
    Ok(())
}

#[test]
fn it_never_replaces_a_rooted_profile_with_another_account() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let profiles = NativeProfileStore::at(tmp.path().join("state"));
    let profile = profiles.create_pending_with_bytes(Some("personal"), [0xa1; 16])?;
    profiles.record_account_root(&profile.id, "did:key:root-a", None)?;

    let error = profiles
        .record_account_root(&profile.id, "did:key:root-b", None)
        .expect_err("a rooted profile is immutable");
    assert!(
        error
            .to_string()
            .contains("this profile belongs to did:key:root-a")
    );
    assert_eq!(
        profiles
            .context(&profile.id)?
            .record
            .account_root
            .as_deref(),
        Some("did:key:root-a")
    );
    Ok(())
}

#[test]
fn it_resumes_an_unrooted_selected_profile_instead_of_creating_another() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let profiles = NativeProfileStore::at(tmp.path().join("state"));
    let pending = profiles.create_pending_with_bytes(Some("work"), [0xb1; 16])?;

    let resumed = profiles.create_or_resume_pending(Some("work"))?;

    assert_eq!(resumed.id, pending.id);
    assert_eq!(resumed.store.root(), pending.store.root());
    assert_eq!(profiles.load_or_bootstrap()?.profiles.len(), 1);
    Ok(())
}

#[test]
fn it_persists_deployment_defaults_only_on_the_target_profile() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let profiles = NativeProfileStore::at(tmp.path().join("state"));
    let personal = profiles.create_pending_with_bytes(Some("personal"), [0xd1; 16])?;
    let work = profiles.create_pending_with_bytes(Some("work"), [0xd2; 16])?;
    let defaults = tonk_cli::deployment::DeploymentDefaults {
        ceremony_origin: "https://personal.example/".parse()?,
        access_remote: "https://personal.example/ucan/".parse()?,
        revocation_relay: "https://relay.example/revocations/".parse()?,
    };

    profiles.record_deployment_defaults(&personal.id, &defaults)?;

    let personal = profiles.context(&personal.id)?;
    let work = profiles.context(&work.id)?;
    assert_eq!(
        personal.record.default_access_remote.as_deref(),
        Some("https://personal.example/ucan/")
    );
    assert_eq!(
        personal.record.default_revocation_relay.as_deref(),
        Some("https://relay.example/revocations/")
    );
    assert!(work.record.default_access_remote.is_none());
    assert!(work.record.default_revocation_relay.is_none());
    Ok(())
}
