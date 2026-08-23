//! Reading the account directory — the one query the worker's adoption
//! and the CLI's `account spots` both run; each renders it its own way.
//!
//! The directory lives on the account branch as plain facts anchored on
//! each space's own entity: the `tonk:space` row ([`Space`]), the
//! [`SpaceName`] mirror, and the mount records ([`Remote`] /
//! [`RemoteExecution`] / [`Branch`](BranchConcept) / [`TrackingBranch`])
//! the configure flow writes. This module is the read side, shaped so a
//! consumer can list spaces cheaply and fetch one space's full mount
//! record when it needs to replicate.

use crate::domain::branch::Origin as BranchOrigin;
use crate::domain::remote::{Address as RemoteAddress, Origin as RemoteOrigin};
use crate::prelude::DidExt as _;
use crate::{
    Branch as BranchConcept, Remote, RemoteExecution, Replica, Space, SpaceName, TrackingBranch,
};
use dialog_artifacts::Entity;
use dialog_capability::{Fork, Provider};
use dialog_common::ConditionalSync;
use dialog_effects::archive::{Get, Import, Put};
use dialog_effects::authority::{Attest, Identify};
use dialog_effects::memory::{Publish, Resolve};
use dialog_query::{EvaluationError, Output as _, Query, Term};
use dialog_repository::{Branch, CommitError, RemoteSite, SiteAddress};
use dialog_varsig::Did;

/// One directory row, as the Hub and the CLI list it.
#[derive(Debug, Clone)]
pub struct DirectorySpace {
    /// The space's subject DID.
    pub subject: Did,
    /// The mirrored display name, when one was recorded.
    pub name: Option<String>,
    /// The seeding status entity URI (`tonk:blank` / `tonk:initialized`).
    pub status: String,
    /// Whether mount records exist — a space another device can
    /// replicate. `false` marks a row that is listed but (currently)
    /// unmountable, e.g. one recorded local-only.
    pub mountable: bool,
}

/// One remote in a space's mount record.
#[derive(Debug, Clone, PartialEq)]
pub struct MountRemote {
    /// The remote's local name (`origin` by convention).
    pub name: String,
    /// The remote's site address.
    pub address: SiteAddress,
    /// The repository subject on the remote side.
    pub subject: Did,
    /// The revocation relay, when recorded.
    pub revocation: Option<String>,
}

/// One branch in a space's mount record.
#[derive(Debug, Clone, PartialEq)]
pub struct MountBranch {
    /// The branch name.
    pub name: String,
    /// `(remote name, branch name)` this branch tracks, if any.
    pub upstream: Option<(String, String)>,
}

/// Everything needed to mount a space exactly as it was configured.
#[derive(Debug, Clone, PartialEq)]
pub struct MountRecord {
    /// The space's remotes.
    pub remotes: Vec<MountRemote>,
    /// The space's branches and their tracking links.
    pub branches: Vec<MountBranch>,
}

/// Anchor wrapper so branch concepts can hang off the space's
/// directory entity (`subject.this()`), giving every device the same
/// derived entities.
struct DirectoryAnchor(Entity);

impl AsRef<Entity> for DirectoryAnchor {
    fn as_ref(&self) -> &Entity {
        &self.0
    }
}

/// Write one space's directory entry — the write side of [`spaces`] /
/// [`mount_record`]: the `tonk:space` row (recorded as initialized —
/// the writer holds a real, seeded replica), the optional [`SpaceName`]
/// mirror, and the full mount record. Every fact re-derives the same
/// entities from `(subject, name)`, so devices converge on one record
/// per space and re-recording an unchanged configuration is
/// idempotent.
///
/// The worker's `record_space_mount` writes the same facts through its
/// reactor-cached branch handle; keep the two shapes in lockstep.
pub async fn record<Env>(
    account: &Branch,
    subject: &Did,
    name: Option<&str>,
    record: &MountRecord,
    env: &Env,
) -> Result<(), CommitError>
where
    Env: Provider<Get>
        + Provider<Put>
        + Provider<Import>
        + Provider<Resolve>
        + Provider<Publish>
        + Provider<Identify>
        + Provider<Attest>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Resolve>>
        + ConditionalSync
        + 'static,
{
    let anchor_entity = subject.this();
    let anchor = DirectoryAnchor(anchor_entity.clone());
    let mut transaction = account
        .transaction()
        .assert(Space::new(subject, Replica::initialized_status()));
    if let Some(name) = name {
        transaction = transaction.assert(SpaceName::new(subject, name));
    }
    let mut remote_concepts: std::collections::HashMap<String, Remote> =
        std::collections::HashMap::new();
    for remote in &record.remotes {
        let concept = Remote::at(
            &anchor_entity,
            remote.subject.clone(),
            RemoteAddress::encode(&remote.address),
            remote.name.as_str(),
        );
        transaction = transaction.assert(concept.clone());
        if let Some(revocation) = &remote.revocation {
            transaction = transaction.assert(RemoteExecution::new(&concept, revocation.as_str()));
        }
        remote_concepts.insert(remote.name.clone(), concept);
    }
    for branch in &record.branches {
        let local = BranchConcept::new(&anchor, branch.name.as_str());
        transaction = transaction.assert(local.clone());
        if let Some((remote_name, upstream_branch)) = &branch.upstream
            && let Some(remote_concept) = remote_concepts.get(remote_name)
        {
            let upstream = BranchConcept::new(remote_concept, upstream_branch.as_str());
            transaction = transaction
                .assert(upstream.clone())
                .assert(TrackingBranch::new(&local, &upstream));
        }
    }
    transaction.commit().perform(env).await?;
    Ok(())
}

/// Every space the directory lists, with names and mountability.
pub async fn spaces<Env>(
    account: &Branch,
    env: &Env,
) -> Result<Vec<DirectorySpace>, EvaluationError>
where
    Env: Provider<Get>
        + Provider<Put>
        + Provider<Resolve>
        + Provider<Identify>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Resolve>>
        + ConditionalSync
        + 'static,
{
    let rows: Vec<Space> = account
        .query()
        .select(Query::<Space> {
            this: Term::var("this"),
            subject: Term::var("subject"),
            status: Term::var("status"),
        })
        .perform(env)
        .try_vec()
        .await?;
    let names: Vec<SpaceName> = account
        .query()
        .select(Query::<SpaceName> {
            this: Term::var("this"),
            name: Term::var("name"),
        })
        .perform(env)
        .try_vec()
        .await?;
    let remotes: Vec<Remote> = account
        .query()
        .select(Query::<Remote> {
            this: Term::var("this"),
            name: Term::var("name"),
            origin: Term::var("origin"),
            subject: Term::var("subject"),
            address: Term::var("address"),
        })
        .perform(env)
        .try_vec()
        .await?;

    let named: std::collections::HashMap<String, String> = names
        .into_iter()
        .map(|row| (row.this.to_string(), row.name.0))
        .collect();
    let mountable: std::collections::HashSet<String> = remotes
        .into_iter()
        .map(|row| row.origin.0.to_string())
        .collect();

    let mut spaces = Vec::with_capacity(rows.len());
    for row in rows {
        let Ok(subject) = row.subject.0.to_string().parse::<Did>() else {
            continue;
        };
        let anchor = row.this.to_string();
        spaces.push(DirectorySpace {
            name: named.get(&anchor).cloned(),
            status: row.status.0.to_string(),
            mountable: mountable.contains(&anchor),
            subject,
        });
    }
    spaces.sort_by(|left, right| left.subject.to_string().cmp(&right.subject.to_string()));
    Ok(spaces)
}

/// Every distinct access-service endpoint the directory's remotes point
/// at.
///
/// One query across the whole account branch rather than a walk per
/// space: a revocation has to reach each service that could still serve
/// the withdrawn authority, and several spaces usually share one.
///
/// Remotes whose address is unreadable or not a UCAN site are skipped —
/// nothing can be published to them, and one bad row must not stop the
/// rest from being told.
pub async fn access_endpoints<Env>(
    account: &Branch,
    env: &Env,
) -> Result<std::collections::BTreeSet<String>, EvaluationError>
where
    Env: Provider<Get>
        + Provider<Put>
        + Provider<Resolve>
        + Provider<Identify>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Resolve>>
        + ConditionalSync
        + 'static,
{
    let remotes: Vec<Remote> = account
        .query()
        .select(Query::<Remote> {
            this: Term::var("this"),
            name: Term::var("name"),
            origin: Term::var("origin"),
            subject: Term::var("subject"),
            address: Term::var("address"),
        })
        .perform(env)
        .try_vec()
        .await?;

    Ok(remotes
        .into_iter()
        .filter_map(|row| match RemoteAddress::decode(&row.address) {
            Ok(SiteAddress::Ucan(ucan)) => Some(ucan.endpoint().to_string()),
            _ => None,
        })
        .collect())
}

/// The full mount record for one space, or `None` when the directory
/// holds no remotes for it.
pub async fn mount_record<Env>(
    account: &Branch,
    subject: &Did,
    env: &Env,
) -> Result<Option<MountRecord>, EvaluationError>
where
    Env: Provider<Get>
        + Provider<Put>
        + Provider<Resolve>
        + Provider<Identify>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Resolve>>
        + ConditionalSync
        + 'static,
{
    let anchor = subject.this();
    let remote_rows: Vec<Remote> = account
        .query()
        .select(Query::<Remote> {
            this: Term::var("this"),
            name: Term::var("name"),
            origin: Term::from(RemoteOrigin::from(anchor.clone())),
            subject: Term::var("subject"),
            address: Term::var("address"),
        })
        .perform(env)
        .try_vec()
        .await?;
    if remote_rows.is_empty() {
        return Ok(None);
    }

    let mut remotes = Vec::with_capacity(remote_rows.len());
    let mut remote_names: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for row in &remote_rows {
        let Ok(address) = RemoteAddress::decode(&row.address) else {
            continue;
        };
        let target: Did = match row.subject.0.to_string().parse() {
            Ok(did) => did,
            Err(_) => subject.clone(),
        };
        let executions: Vec<RemoteExecution> = account
            .query()
            .select(Query::<RemoteExecution> {
                this: Term::from(row.this.clone()),
                revocation_url: Term::var("revocation_url"),
            })
            .perform(env)
            .try_vec()
            .await
            .unwrap_or_default();
        remotes.push(MountRemote {
            name: row.name.0.clone(),
            address,
            subject: target,
            revocation: executions
                .into_iter()
                .next()
                .map(|execution| execution.revocation_url.0),
        });
        remote_names.insert(row.this.to_string(), row.name.0.clone());
    }

    let locals: Vec<BranchConcept> = account
        .query()
        .select(Query::<BranchConcept> {
            this: Term::var("this"),
            name: Term::var("name"),
            origin: Term::from(BranchOrigin::from(anchor.clone())),
        })
        .perform(env)
        .try_vec()
        .await
        .unwrap_or_default();
    // Upstream branch entities are anchored on their remote concept.
    let mut remote_branches: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new();
    for (remote_entity, remote_name) in &remote_names {
        let Ok(origin) = remote_entity.parse::<dialog_artifacts::Entity>() else {
            continue;
        };
        let rows: Vec<BranchConcept> = account
            .query()
            .select(Query::<BranchConcept> {
                this: Term::var("this"),
                name: Term::var("name"),
                origin: Term::from(BranchOrigin::from(origin)),
            })
            .perform(env)
            .try_vec()
            .await
            .unwrap_or_default();
        for row in rows {
            remote_branches.insert(
                row.this.to_string(),
                (remote_name.clone(), row.name.0.clone()),
            );
        }
    }
    let mut branches = Vec::with_capacity(locals.len());
    for local in locals {
        let tracking: Vec<TrackingBranch> = account
            .query()
            .select(Query::<TrackingBranch> {
                this: Term::from(local.this.clone()),
                upstream: Term::var("upstream"),
                origin: Term::var("origin"),
            })
            .perform(env)
            .try_vec()
            .await
            .unwrap_or_default();
        branches.push(MountBranch {
            name: local.name.0.clone(),
            upstream: tracking
                .into_iter()
                .next()
                .and_then(|link| remote_branches.get(&link.upstream.0.to_string()).cloned()),
        });
    }
    Ok(Some(MountRecord { remotes, branches }))
}
