//! Root-owned facts stored in the hidden account repository.

use std::collections::BTreeMap;

use base58::FromBase58 as _;
use dialog_artifacts::Entity;
use dialog_capability::Provider;
use dialog_effects::archive::Import;
use dialog_effects::authority::Attest;
use dialog_effects::memory::Publish;
use dialog_query::Concept;
use dialog_query::{Output as _, Query, Term};
use dialog_repository::Branch;
use dialog_varsig::Did;
use serde::Serialize;

use crate::concept::QueryEnv;
use crate::domain::account::{
    Archived, DisplayName, Name, PasskeyCreatedAt, PasskeyCreatedOn, Relay, Remote, Root, Space,
    Tree,
};
use crate::prelude::{DidExt as _, EntityExt as _};

/// The account-wide display name, keyed by the immutable account subject.
///
/// The name is cardinality-one. Concurrent linked-device writes therefore
/// converge to one deterministic value; no wall-clock latest-write ordering is
/// implied.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AccountDisplayName {
    /// The immutable account subject.
    pub this: Entity,
    /// The authoritative account display name.
    pub name: DisplayName,
}

impl AccountDisplayName {
    /// Name the account subject.
    pub fn new(account: Entity, name: String) -> Self {
        Self {
            this: account,
            name: DisplayName(name),
        }
    }
}

/// Facts Tonk recorded when it created this account's passkey, keyed by the
/// immutable account subject.
///
/// Informational only: no derivation, delegation, authorization, or revocation
/// path reads these. Both attributes are asserted in one transaction, so a
/// query requiring both never observes a half-written pair on one replica.
///
/// Merge is per attribute, not per concept: two replicas that recorded
/// *different* pairs converge on one value for each attribute independently,
/// which can pair one device's clock with another device's label. Only the
/// browser that ran `navigator.credentials.create()` ever records this
/// metadata — evaluating an existing passkey carries none — so one account has
/// at most one pair to converge and that mismatch has no way to arise. A
/// second recorded pair per account would need this keyed on the credential
/// instead of the account, which is where per-credential modelling belongs.
///
/// Derives `PartialOrd` but not `Ord`, because [`PasskeyCreatedAt`] wraps an
/// `f64` — the same shape `command::Invite` uses for its `TimeStamp`.
#[derive(Concept, Debug, Clone, PartialEq, PartialOrd)]
pub struct AccountPasskeyCreated {
    /// The immutable account subject.
    pub this: Entity,
    /// Unix seconds at credential creation.
    pub created_at: PasskeyCreatedAt,
    /// Browser and operating-system label where creation ran.
    pub created_on: PasskeyCreatedOn,
}

impl AccountPasskeyCreated {
    /// Record creation facts on the account subject.
    pub fn new(account: Entity, created_at: u64, created_on: String) -> Self {
        Self {
            this: account,
            created_at: PasskeyCreatedAt(created_at as f64),
            created_on: PasskeyCreatedOn(created_on),
        }
    }

    /// Unix seconds, back in the integer form the wire DTO carries.
    pub fn seconds(&self) -> u64 {
        self.created_at.0 as u64
    }
}

/// Canonical record that an account has known one repository subject.
///
/// The entity is derived from the immutable `(account root, repository
/// subject)` pair so every client and replica converges on the same fact.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AccountSpace {
    /// Entity derived from the account and repository DIDs.
    pub this: Entity,
    /// Immutable account root.
    pub account: Root,
    /// Immutable repository subject.
    pub subject: Space,
}

#[derive(Debug, Clone, Serialize)]
enum AccountSpaceEntity<'a> {
    AccountSpace { account: &'a Did, subject: &'a Did },
}

impl AccountSpace {
    /// Derive the canonical membership entity for an account and repository.
    pub fn new(account: Did, subject: Did) -> Self {
        Self {
            this: Entity::of(&AccountSpaceEntity::AccountSpace {
                account: &account,
                subject: &subject,
            }),
            account: Root(account.this()),
            subject: Space(subject.this()),
        }
    }
}

/// Optional display name for an [`AccountSpace`].
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AccountSpaceName {
    /// Account-space entity being named.
    pub this: Entity,
    /// Current account-facing name.
    pub name: Name,
}

/// Optional content remote for an [`AccountSpace`].
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AccountSpaceRemote {
    /// Account-space entity being described.
    pub this: Entity,
    /// Current content remote URL.
    pub remote: Remote,
}

/// Optional invitation-revocation relay for an [`AccountSpace`].
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AccountSpaceRevocationRelay {
    /// Account-space entity being described.
    pub this: Entity,
    /// Current relay URL.
    pub relay: Relay,
}

/// Exact content revision confirmed by the remote for an [`AccountSpace`].
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AccountSpaceConfirmedRevision {
    /// Account-space entity being described.
    pub this: Entity,
    /// Exact confirmed tree reference.
    pub tree: Tree,
}

/// Monotonic archive stamp for an [`AccountSpace`].
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AccountSpaceArchived {
    /// Account-space entity being archived.
    pub this: Entity,
    /// Presence marker; consumers treat any row as archived.
    pub marker: Archived,
}

/// Raw account-space query rows before they are joined by canonical entity.
#[derive(Clone, Debug, Default)]
pub struct AccountSpaceRows {
    /// Base membership facts.
    pub spaces: Vec<AccountSpace>,
    /// Optional display-name stamps.
    pub names: Vec<AccountSpaceName>,
    /// Optional remote stamps.
    pub remotes: Vec<AccountSpaceRemote>,
    /// Optional revocation-relay stamps.
    pub relays: Vec<AccountSpaceRevocationRelay>,
    /// Optional exact confirmed-tree stamps.
    pub confirmed_revisions: Vec<AccountSpaceConfirmedRevision>,
    /// Monotonic archive stamps.
    pub archives: Vec<AccountSpaceArchived>,
}

/// One normalized canonical account-space record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountSpaceRecord {
    /// Immutable account root.
    pub account: Did,
    /// Immutable repository subject.
    pub subject: Did,
    /// Current account-facing name.
    pub name: Option<String>,
    /// Current content remote URL.
    pub remote_url: Option<String>,
    /// Current invitation-revocation relay URL.
    pub revocation_url: Option<String>,
    /// Exact tree last accepted by the content remote.
    pub confirmed_revision: Option<String>,
    /// Whether a monotonic archive marker exists.
    pub archived: bool,
}

/// Metadata to assert alongside one canonical active membership fact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountSpaceInput {
    /// Immutable account root.
    pub account: Did,
    /// Immutable repository subject.
    pub subject: Did,
    /// Account-facing display name to update, when known.
    pub name: Option<String>,
    /// Content remote URL to update, when known.
    pub remote_url: Option<String>,
    /// Invitation-revocation relay URL to update, when known.
    pub revocation_url: Option<String>,
    /// Exact confirmed content tree to update, when known.
    pub confirmed_revision: Option<String>,
}

#[derive(Default)]
struct PartialAccountSpaceRecord {
    account: Option<Did>,
    subject: Option<Did>,
    name: Option<String>,
    remote_url: Option<String>,
    revocation_url: Option<String>,
    confirmed_revision: Option<String>,
    archived: bool,
}

/// Stable failures while normalizing canonical account-space facts.
#[derive(Debug, thiserror::Error)]
pub enum AccountSpaceError {
    /// A stored base row does not match its deterministic entity.
    #[error("invalid account-space fact: {0}")]
    InvalidFact(String),
    /// A branch query failed.
    #[error("failed to query account-space facts: {0}")]
    Query(String),
    /// A fact commit failed.
    #[error("failed to commit account-space facts: {0}")]
    Commit(String),
    /// Monotonic archive state forbids reactivation.
    #[error("account space {subject} is permanently archived")]
    Archived {
        /// Repository subject that cannot be reactivated.
        subject: Did,
    },
    /// Archive was requested for an account/subject pair with no base fact.
    #[error("account space {subject} is not known to account {account}")]
    Unknown {
        /// Account root searched.
        account: Did,
        /// Repository subject searched.
        subject: Did,
    },
}

/// Join independently queried account-space concepts by their deterministic
/// entity. Archive presence always dominates metadata and input ordering.
pub fn normalize_account_spaces(
    rows: AccountSpaceRows,
) -> Result<Vec<AccountSpaceRecord>, AccountSpaceError> {
    let mut records: BTreeMap<Entity, PartialAccountSpaceRecord> = BTreeMap::new();
    for space in rows.spaces {
        let account = parse_did(&space.account.0, "account root")?;
        let subject = parse_did(&space.subject.0, "repository subject")?;
        let expected = AccountSpace::new(account.clone(), subject.clone()).this;
        if expected != space.this {
            return Err(AccountSpaceError::InvalidFact(format!(
                "entity {} does not match account {} and subject {}",
                space.this, account, subject
            )));
        }
        let record = records.entry(space.this).or_default();
        record.account = Some(account);
        record.subject = Some(subject);
    }
    for row in rows.names {
        records.entry(row.this).or_default().name = Some(row.name.0);
    }
    for row in rows.remotes {
        records.entry(row.this).or_default().remote_url = Some(row.remote.0);
    }
    for row in rows.relays {
        records.entry(row.this).or_default().revocation_url = Some(row.relay.0);
    }
    for row in rows.confirmed_revisions {
        validate_tree_reference(&row.tree.0)?;
        records.entry(row.this).or_default().confirmed_revision = Some(row.tree.0);
    }
    for row in rows.archives {
        records.entry(row.this).or_default().archived = true;
    }

    let mut normalized = records
        .into_values()
        .filter_map(|record| {
            Some(AccountSpaceRecord {
                account: record.account?,
                subject: record.subject?,
                name: record.name,
                remote_url: record.remote_url,
                revocation_url: record.revocation_url,
                confirmed_revision: record.confirmed_revision,
                archived: record.archived,
            })
        })
        .collect::<Vec<_>>();
    normalized.sort_by(|left, right| left.subject.cmp(&right.subject));
    Ok(normalized)
}

fn parse_did(entity: &Entity, label: &str) -> Result<Did, AccountSpaceError> {
    entity.to_string().parse().map_err(|error| {
        AccountSpaceError::InvalidFact(format!("{label} {entity} is not a DID: {error}"))
    })
}

fn validate_tree_reference(value: &str) -> Result<(), AccountSpaceError> {
    let encoded = value.strip_prefix('#').ok_or_else(|| {
        AccountSpaceError::InvalidFact(format!(
            "confirmed revision '{value}' is not a tree reference"
        ))
    })?;
    let decoded = encoded.from_base58().map_err(|error| {
        AccountSpaceError::InvalidFact(format!(
            "confirmed revision '{value}' is not base58: {error:?}"
        ))
    })?;
    if decoded.len() != 32 {
        return Err(AccountSpaceError::InvalidFact(format!(
            "confirmed revision '{value}' has {} bytes, expected 32",
            decoded.len()
        )));
    }
    Ok(())
}

/// Query and normalize all canonical account-space rows on a branch.
pub async fn list_account_spaces<Env: QueryEnv>(
    branch: &Branch,
    env: &Env,
) -> Result<Vec<AccountSpaceRecord>, AccountSpaceError> {
    let spaces = branch
        .query()
        .select(Query::<AccountSpace> {
            this: Term::var("this"),
            account: Term::var("account"),
            subject: Term::var("subject"),
        })
        .perform(env)
        .try_vec()
        .await
        .map_err(|error| AccountSpaceError::Query(format!("base rows: {error:?}")))?;
    let names = branch
        .query()
        .select(Query::<AccountSpaceName> {
            this: Term::var("this"),
            name: Term::var("name"),
        })
        .perform(env)
        .try_vec()
        .await
        .map_err(|error| AccountSpaceError::Query(format!("name rows: {error:?}")))?;
    let remotes = branch
        .query()
        .select(Query::<AccountSpaceRemote> {
            this: Term::var("this"),
            remote: Term::var("remote"),
        })
        .perform(env)
        .try_vec()
        .await
        .map_err(|error| AccountSpaceError::Query(format!("remote rows: {error:?}")))?;
    let relays = branch
        .query()
        .select(Query::<AccountSpaceRevocationRelay> {
            this: Term::var("this"),
            relay: Term::var("relay"),
        })
        .perform(env)
        .try_vec()
        .await
        .map_err(|error| AccountSpaceError::Query(format!("relay rows: {error:?}")))?;
    let confirmed_revisions = branch
        .query()
        .select(Query::<AccountSpaceConfirmedRevision> {
            this: Term::var("this"),
            tree: Term::var("tree"),
        })
        .perform(env)
        .try_vec()
        .await
        .map_err(|error| AccountSpaceError::Query(format!("confirmed rows: {error:?}")))?;
    let archives = branch
        .query()
        .select(Query::<AccountSpaceArchived> {
            this: Term::var("this"),
            marker: Term::var("marker"),
        })
        .perform(env)
        .try_vec()
        .await
        .map_err(|error| AccountSpaceError::Query(format!("archive rows: {error:?}")))?;

    normalize_account_spaces(AccountSpaceRows {
        spaces,
        names,
        remotes,
        relays,
        confirmed_revisions,
        archives,
    })
}

/// Assert active membership and only the metadata supplied by this call.
/// Existing optional metadata is preserved; an archive marker is never removed.
pub async fn record_active_account_space<Env>(
    branch: &Branch,
    input: AccountSpaceInput,
    env: &Env,
) -> Result<bool, AccountSpaceError>
where
    Env: QueryEnv + Provider<Publish> + Provider<Import> + Provider<Attest>,
{
    if let Some(tree) = input.confirmed_revision.as_deref() {
        validate_tree_reference(tree)?;
    }
    if list_account_spaces(branch, env)
        .await?
        .iter()
        .any(|record| {
            record.account == input.account && record.subject == input.subject && record.archived
        })
    {
        return Err(AccountSpaceError::Archived {
            subject: input.subject,
        });
    }

    let space = AccountSpace::new(input.account, input.subject);
    let this = space.this.clone();
    let mut transaction = branch.transaction().assert(space);
    if let Some(name) = input.name {
        transaction = transaction.assert(AccountSpaceName {
            this: this.clone(),
            name: Name(name),
        });
    }
    if let Some(remote) = input.remote_url {
        transaction = transaction.assert(AccountSpaceRemote {
            this: this.clone(),
            remote: Remote(remote),
        });
    }
    if let Some(relay) = input.revocation_url {
        transaction = transaction.assert(AccountSpaceRevocationRelay {
            this: this.clone(),
            relay: Relay(relay),
        });
    }
    if let Some(tree) = input.confirmed_revision {
        transaction = transaction.assert(AccountSpaceConfirmedRevision {
            this,
            tree: Tree(tree),
        });
    }
    transaction
        .commit()
        .perform(env)
        .await
        .map_err(|error| AccountSpaceError::Commit(error.to_string()))?;
    Ok(true)
}

/// Add the monotonic archive marker for an exact account/subject pair.
/// Returns `false` when that marker already exists.
pub async fn archive_account_space<Env>(
    branch: &Branch,
    account: &Did,
    subject: &Did,
    env: &Env,
) -> Result<bool, AccountSpaceError>
where
    Env: QueryEnv + Provider<Publish> + Provider<Import> + Provider<Attest>,
{
    let existing = list_account_spaces(branch, env).await?;
    let Some(record) = existing
        .iter()
        .find(|record| &record.account == account && &record.subject == subject)
    else {
        return Err(AccountSpaceError::Unknown {
            account: account.clone(),
            subject: subject.clone(),
        });
    };
    if record.archived {
        return Ok(false);
    }

    let this = AccountSpace::new(account.clone(), subject.clone()).this;
    branch
        .transaction()
        .assert(AccountSpaceArchived {
            this,
            marker: Archived(true),
        })
        .commit()
        .perform(env)
        .await
        .map_err(|error| AccountSpaceError::Commit(error.to_string()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use dialog_operator::helpers;
    use dialog_query::{Output as _, Query, Term};
    use dialog_varsig::did;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    use super::*;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    fn tree(byte: u8) -> String {
        dialog_artifacts::TreeReference::from([byte; 32]).to_string()
    }

    async fn converge(a_first: bool) -> Result<String> {
        let (operator, profile) = helpers::test_operator_with_profile().await;
        let repository = helpers::test_repo(&operator, &profile).await;
        let base = repository.branch("base").open().perform(&operator).await?;
        let base_revision = base.transaction().commit().perform(&operator).await?;
        let a = repository
            .branch("replica-a")
            .open()
            .perform(&operator)
            .await?;
        let b = repository
            .branch("replica-b")
            .open()
            .perform(&operator)
            .await?;
        a.reset(base_revision.clone()).perform(&operator).await?;
        b.reset(base_revision).perform(&operator).await?;
        a.set_upstream(&b).perform(&operator).await?;
        b.set_upstream(&a).perform(&operator).await?;

        let account = did!("test:account").this();
        a.transaction()
            .assert(AccountDisplayName::new(account.clone(), "Amber".into()))
            .commit()
            .perform(&operator)
            .await?;
        b.transaction()
            .assert(AccountDisplayName::new(account.clone(), "Violet".into()))
            .commit()
            .perform(&operator)
            .await?;

        if a_first {
            a.pull().perform(&operator).await?;
            b.pull().perform(&operator).await?;
        } else {
            b.pull().perform(&operator).await?;
            a.pull().perform(&operator).await?;
        }
        a.pull().perform(&operator).await?;
        b.pull().perform(&operator).await?;

        let a_rows: Vec<AccountDisplayName> = a
            .query()
            .select(Query::<AccountDisplayName> {
                this: Term::from(account.clone()),
                name: Term::var("name"),
            })
            .perform(&operator)
            .try_vec()
            .await?;
        let b_rows: Vec<AccountDisplayName> = b
            .query()
            .select(Query::<AccountDisplayName> {
                this: Term::from(account.clone()),
                name: Term::var("name"),
            })
            .perform(&operator)
            .try_vec()
            .await?;
        let a_value = a_rows.into_iter().next().expect("one account name").name.0;
        let b_value = b_rows.into_iter().next().expect("one account name").name.0;
        assert_eq!(a_value, b_value);

        b.transaction()
            .assert(AccountDisplayName::new(account.clone(), "Cedar".into()))
            .commit()
            .perform(&operator)
            .await?;
        a.pull().perform(&operator).await?;
        let rows: Vec<AccountDisplayName> = a
            .query()
            .select(Query::<AccountDisplayName> {
                this: Term::from(account),
                name: Term::var("name"),
            })
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(
            rows.into_iter().next().expect("one account name").name.0,
            "Cedar"
        );
        Ok(a_value)
    }

    #[dialog_common::test]
    async fn it_converges_divergent_display_names_in_both_orders() -> Result<()> {
        let a_then_b = converge(true).await?;
        let b_then_a = converge(false).await?;

        // Order independence is the property that matters, and it is the one
        // this pins. Which of two concurrent names wins is dialog's
        // cardinality-one merge to decide, not wall-clock latest-write, so
        // asserting the specific winner would only pin that internal choice.
        assert_eq!(a_then_b, b_then_a);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_round_trips_passkey_creation_facts_on_the_account_subject() -> Result<()> {
        let (operator, profile) = helpers::test_operator_with_profile().await;
        let repository = helpers::test_repo(&operator, &profile).await;
        let branch = repository.branch("main").open().perform(&operator).await?;
        let account = did!("test:account").this();

        branch
            .transaction()
            .assert(AccountPasskeyCreated::new(
                account.clone(),
                1_754_380_800,
                "Chrome on macOS".into(),
            ))
            .commit()
            .perform(&operator)
            .await?;

        let rows: Vec<AccountPasskeyCreated> = branch
            .query()
            .select(Query::<AccountPasskeyCreated> {
                this: Term::from(account),
                created_at: Term::var("created_at"),
                created_on: Term::var("created_on"),
            })
            .perform(&operator)
            .try_vec()
            .await?;

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].seconds(), 1_754_380_800);
        assert_eq!(rows[0].created_on.0, "Chrome on macOS");
        Ok(())
    }

    #[test]
    fn it_derives_one_account_space_entity_per_root_and_subject() {
        let first = AccountSpace::new(did!("test:account-a"), did!("test:space-a"));
        let same = AccountSpace::new(did!("test:account-a"), did!("test:space-a"));
        let other_account = AccountSpace::new(did!("test:account-b"), did!("test:space-a"));
        let other_subject = AccountSpace::new(did!("test:account-a"), did!("test:space-b"));

        assert_eq!(first.this, same.this);
        assert_ne!(first.this, other_account.this);
        assert_ne!(first.this, other_subject.this);
        assert_eq!(first.account.0.to_string(), "did:test:account-a");
        assert_eq!(first.subject.0.to_string(), "did:test:space-a");
    }

    #[test]
    fn it_assembles_active_and_archived_records_independently_of_query_order() {
        let account = did!("test:account");
        let active = AccountSpace::new(account.clone(), did!("test:active"));
        let archived = AccountSpace::new(account, did!("test:archived"));
        let rows = AccountSpaceRows {
            spaces: vec![active.clone(), archived.clone()],
            names: vec![AccountSpaceName {
                this: active.this.clone(),
                name: Name("Garden".to_string()),
            }],
            remotes: vec![],
            relays: vec![],
            confirmed_revisions: vec![AccountSpaceConfirmedRevision {
                this: archived.this.clone(),
                tree: Tree(tree(1)),
            }],
            archives: vec![AccountSpaceArchived {
                this: archived.this,
                marker: Archived(true),
            }],
        };
        let mut reversed = rows.clone();
        reversed.spaces.reverse();
        reversed.names.reverse();
        reversed.confirmed_revisions.reverse();
        reversed.archives.reverse();

        let forward = normalize_account_spaces(rows).unwrap();
        let reverse = normalize_account_spaces(reversed).unwrap();

        assert_eq!(forward, reverse);
        assert_eq!(forward.len(), 2);
        assert!(!forward[0].archived);
        assert!(forward[1].archived);
        assert_eq!(forward[1].confirmed_revision, Some(tree(1)));
    }

    #[dialog_common::test]
    async fn it_refuses_to_reactivate_an_archived_subject() -> Result<()> {
        let (operator, profile) = helpers::test_operator_with_profile().await;
        let repository = helpers::test_repo(&operator, &profile).await;
        let branch = repository.branch("main").open().perform(&operator).await?;
        let input = AccountSpaceInput {
            account: did!("test:account"),
            subject: did!("test:space"),
            name: Some("Garden".to_string()),
            remote_url: None,
            revocation_url: None,
            confirmed_revision: Some(tree(2)),
        };

        record_active_account_space(&branch, input.clone(), &operator).await?;
        assert!(archive_account_space(&branch, &input.account, &input.subject, &operator).await?);
        let error = record_active_account_space(&branch, input, &operator)
            .await
            .unwrap_err();

        assert!(matches!(error, AccountSpaceError::Archived { .. }));
        let records = list_account_spaces(&branch, &operator).await?;
        assert!(records[0].archived);
        assert_eq!(records[0].confirmed_revision, Some(tree(2)));
        Ok(())
    }

    #[dialog_common::test]
    async fn it_archives_idempotently() -> Result<()> {
        let (operator, profile) = helpers::test_operator_with_profile().await;
        let repository = helpers::test_repo(&operator, &profile).await;
        let branch = repository.branch("main").open().perform(&operator).await?;
        let account = did!("test:account");
        let subject = did!("test:space");
        record_active_account_space(
            &branch,
            AccountSpaceInput {
                account: account.clone(),
                subject: subject.clone(),
                name: None,
                remote_url: None,
                revocation_url: None,
                confirmed_revision: None,
            },
            &operator,
        )
        .await?;

        assert!(archive_account_space(&branch, &account, &subject, &operator).await?);
        assert!(!archive_account_space(&branch, &account, &subject, &operator).await?);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_converges_an_account_space_archive_across_two_replicas() -> Result<()> {
        let (operator, profile) = helpers::test_operator_with_profile().await;
        let repository = helpers::test_repo(&operator, &profile).await;
        let base = repository.branch("base").open().perform(&operator).await?;
        let base_revision = base.transaction().commit().perform(&operator).await?;
        let a = repository
            .branch("account-space-a")
            .open()
            .perform(&operator)
            .await?;
        let b = repository
            .branch("account-space-b")
            .open()
            .perform(&operator)
            .await?;
        a.reset(base_revision.clone()).perform(&operator).await?;
        b.reset(base_revision).perform(&operator).await?;
        a.set_upstream(&b).perform(&operator).await?;
        b.set_upstream(&a).perform(&operator).await?;

        let account = did!("test:archive-account");
        let subject = did!("test:archive-space");
        record_active_account_space(
            &a,
            AccountSpaceInput {
                account: account.clone(),
                subject: subject.clone(),
                name: Some("archive me".to_string()),
                remote_url: None,
                revocation_url: None,
                confirmed_revision: Some(tree(3)),
            },
            &operator,
        )
        .await?;
        b.pull().perform(&operator).await?;
        assert!(archive_account_space(&b, &account, &subject, &operator).await?);

        a.pull().perform(&operator).await?;
        b.pull().perform(&operator).await?;

        for branch in [&a, &b] {
            let rows = list_account_spaces(branch, &operator).await?;
            let row = rows.first().expect("one account-space row");
            assert!(row.archived, "archive marker must dominate after merge");
            assert_eq!(
                row.confirmed_revision,
                Some(tree(3)),
                "archive keeps the last confirmed revision as history"
            );
        }
        Ok(())
    }

    /// Record `first` on one replica and `second` on the other, exchange them
    /// in the given order, and report the single pair that survived.
    async fn converge_passkey(
        a_first: bool,
        first: (u64, &str),
        second: (u64, &str),
    ) -> Result<(u64, String)> {
        let (operator, profile) = helpers::test_operator_with_profile().await;
        let repository = helpers::test_repo(&operator, &profile).await;
        let base = repository.branch("base").open().perform(&operator).await?;
        let base_revision = base.transaction().commit().perform(&operator).await?;
        let a = repository
            .branch("replica-a")
            .open()
            .perform(&operator)
            .await?;
        let b = repository
            .branch("replica-b")
            .open()
            .perform(&operator)
            .await?;
        a.reset(base_revision.clone()).perform(&operator).await?;
        b.reset(base_revision).perform(&operator).await?;
        a.set_upstream(&b).perform(&operator).await?;
        b.set_upstream(&a).perform(&operator).await?;

        let account = did!("test:account").this();
        a.transaction()
            .assert(AccountPasskeyCreated::new(
                account.clone(),
                first.0,
                first.1.to_string(),
            ))
            .commit()
            .perform(&operator)
            .await?;
        b.transaction()
            .assert(AccountPasskeyCreated::new(
                account.clone(),
                second.0,
                second.1.to_string(),
            ))
            .commit()
            .perform(&operator)
            .await?;

        if a_first {
            a.pull().perform(&operator).await?;
            b.pull().perform(&operator).await?;
        } else {
            b.pull().perform(&operator).await?;
            a.pull().perform(&operator).await?;
        }
        a.pull().perform(&operator).await?;
        b.pull().perform(&operator).await?;

        let rows: Vec<AccountPasskeyCreated> = a
            .query()
            .select(Query::<AccountPasskeyCreated> {
                this: Term::from(account),
                created_at: Term::var("created_at"),
                created_on: Term::var("created_on"),
            })
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(rows.len(), 1, "an account has one passkey creation moment");
        let row = rows.into_iter().next().expect("one creation fact");
        Ok((row.seconds(), row.created_on.0))
    }

    #[dialog_common::test]
    async fn it_keeps_one_passkey_creation_fact_per_account() -> Result<()> {
        let recorded = (1_754_380_800, "Chrome on macOS");

        // Two devices seeding the same recorded pair is the only concurrency
        // this fact can actually see: the seed reads the account space first
        // and only the browser that created the passkey holds metadata to
        // contribute, so every writer contributes the same pair.
        let a_then_b = converge_passkey(true, recorded, recorded).await?;
        let b_then_a = converge_passkey(false, recorded, recorded).await?;
        assert_eq!(a_then_b, (recorded.0, recorded.1.to_string()));
        assert_eq!(b_then_a, (recorded.0, recorded.1.to_string()));

        // Divergent pairs converge on one value per attribute, in an order
        // the merge decides, but *independently* — so the surviving row can
        // pair one write's time with the other's label. Nothing asserts a
        // second pair today; this pins the behaviour that per-credential
        // modelling would have to answer for before one could.
        let divergent = (1_600_000_000, "Safari on iOS");
        assert_eq!(
            converge_passkey(true, recorded, divergent).await?,
            converge_passkey(false, recorded, divergent).await?,
            "concurrent creation facts converge regardless of merge order"
        );
        Ok(())
    }
}
