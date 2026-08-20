//! One account at a time: which replicas the signed-in account may open, and
//! what tonk says about the ones it may not.

use anyhow::Result;
use tonk_cli::spot::{AccountRecord, SpotEntry, SpotError, SpotStore};

const ACCOUNT_A: &str = "did:key:z6MkAccountAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const ACCOUNT_B: &str = "did:key:z6MkAccountBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

fn store_with(
    tmp: &tempfile::TempDir,
    space_account: Option<&str>,
    signed_in: Option<&str>,
) -> Result<SpotStore> {
    let store = SpotStore::at(tmp.path());
    let mut registry = store.load()?;
    registry.spots.insert(
        "garden".to_owned(),
        SpotEntry::at(tmp.path().join("garden")),
    );
    registry.account = signed_in.map(AccountRecord::new);
    store.save(&registry)?;
    if let Some(account) = space_account {
        store.set_space_account("garden", Some(account))?;
    }
    Ok(store)
}

#[test]
fn it_resolves_a_space_that_belongs_to_the_signed_in_account() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let store = store_with(&tmp, Some(ACCOUNT_A), Some(ACCOUNT_A))?;

    let resolved = store.resolve(Some("garden"), None, None)?;

    assert_eq!(resolved.name, "garden");
    Ok(())
}

#[test]
fn it_refuses_a_space_that_belongs_to_another_account() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let store = store_with(&tmp, Some(ACCOUNT_A), Some(ACCOUNT_B))?;

    let error = store
        .resolve(Some("garden"), None, None)
        .expect_err("a foreign space must not resolve");

    assert!(
        matches!(&error, SpotError::ForeignAccount { name, owner, active }
            if name == "garden" && owner == ACCOUNT_A && active == ACCOUNT_B),
        "{error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("this account doesn't have access to 'garden'"),
        "{message}"
    );
    assert!(message.contains("ask its owner for an invite"), "{message}");
    assert!(message.contains("tonk join"), "{message}");
    Ok(())
}

#[test]
fn it_keeps_every_replica_reachable_while_signed_out() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let store = store_with(&tmp, Some(ACCOUNT_A), None)?;

    let resolved = store.resolve(Some("garden"), None, None)?;

    assert_eq!(resolved.name, "garden");
    Ok(())
}

#[test]
fn it_leaves_a_local_only_space_open_to_any_account() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let store = store_with(&tmp, None, Some(ACCOUNT_B))?;

    let resolved = store.resolve(Some("garden"), None, None)?;

    assert_eq!(resolved.name, "garden");
    Ok(())
}

#[test]
fn signing_out_keeps_each_space_with_its_own_account() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let store = store_with(&tmp, Some(ACCOUNT_A), Some(ACCOUNT_A))?;

    store.set_account(None)?;

    let registry = store.load()?;
    assert!(registry.account.is_none());
    assert_eq!(registry.spots["garden"].account.as_deref(), Some(ACCOUNT_A));

    // …and signing into another account makes exactly that space unavailable,
    // without touching what it belongs to.
    store.set_account(Some(AccountRecord::new(ACCOUNT_B)))?;
    assert!(store.resolve(Some("garden"), None, None).is_err());
    assert_eq!(
        store.load()?.spots["garden"].account.as_deref(),
        Some(ACCOUNT_A)
    );
    Ok(())
}

#[test]
fn it_round_trips_the_account_fields_through_the_public_registry_format() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let store = store_with(&tmp, Some(ACCOUNT_A), Some(ACCOUNT_A))?;
    let mut record = AccountRecord::new(ACCOUNT_A);
    record.access_remote = Some("https://example.test/ucan/".to_owned());
    record.revocation_relay = Some("https://example.test/revocations".to_owned());
    store.set_account(Some(record.clone()))?;

    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(store.registry_path())?)?;
    assert_eq!(json["account"]["root"], ACCOUNT_A);
    assert_eq!(
        json["account"]["accessRemote"],
        "https://example.test/ucan/"
    );
    assert_eq!(json["spots"]["garden"]["account"], ACCOUNT_A);
    assert_eq!(store.account()?, Some(record));
    Ok(())
}

#[test]
fn a_local_only_registry_writes_no_account_fields() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let store = store_with(&tmp, None, None)?;

    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(store.registry_path())?)?;

    assert!(json.get("account").is_none(), "{json}");
    assert!(json["spots"]["garden"].get("account").is_none(), "{json}");
    Ok(())
}

#[test]
fn the_already_owned_message_names_the_owner_and_the_way_forward() {
    let message = tonk_cli::space_link::already_owned_message("garden", ACCOUNT_A);

    assert!(message.contains("\"garden\" already belongs to an account"));
    assert!(message.contains("This keeps existing shares working."));
    assert!(message.contains("tonk invite"));
    assert!(message.contains(ACCOUNT_A));
    for forbidden in ["UCAN", "delegation", "prefix"] {
        assert!(!message.contains(forbidden), "copy leaked {forbidden}");
    }
}
