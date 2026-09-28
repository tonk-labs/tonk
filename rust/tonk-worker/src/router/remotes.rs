//! The remotes a repository names, as its meta branch records them.
//!
//! A remote was a name a repository gave to an address and the repository
//! there. Dialog keeps peers as the host's contacts instead, reached by
//! the DID their address names, so the names a repository gives its
//! remotes live only in tonk's own `Remote` records on `meta`, written
//! wherever a remote is configured.

use dialog_capability::Principal;
use dialog_query::{Output as _, Query, Term};
use dialog_repository::{ConnectedReplica, Repository, SiteAddress};
use dialog_varsig::Did;
use tonk_schema::domain::remote as remote_dom;
use tonk_schema::{Remote as RemoteConcept, Replica};

use crate::TonkWorkerError;
use crate::worker::DefaultOperator;

/// A remote a repository names: the repository it is, and where that
/// repository is reached.
#[derive(Debug, Clone)]
pub(crate) struct RecordedRemote {
    /// The repository at the remote.
    pub(crate) subject: Did,
    /// Where it is reached.
    pub(crate) address: SiteAddress,
}

impl RecordedRemote {
    /// Connect to the remote, recording its address among the host's
    /// contacts.
    pub(crate) async fn connect(
        &self,
        operator: &DefaultOperator,
    ) -> Result<ConnectedReplica, TonkWorkerError> {
        tonk_account::peer::connect(self.address.clone(), self.subject.clone(), operator)
            .await
            .map_err(|error| TonkWorkerError::Internal(error.to_string()))
    }

    /// Whether `replica` is this remote: the same repository, at a peer
    /// reached at this remote's address.
    pub(crate) fn is(&self, replica: &ConnectedReplica) -> bool {
        replica.did() == self.subject && replica.addresses().contains(&self.address)
    }
}

/// Every remote `repository` names on this replica, by name.
pub(crate) async fn list<R: Principal>(
    repository: &Repository<R>,
    operator: &DefaultOperator,
) -> Result<Vec<(String, RecordedRemote)>, TonkWorkerError> {
    let replica = Replica::new(operator.home().clone(), repository.did());
    let meta = repository
        .branch(super::repository::META_BRANCH)
        .open()
        .perform(operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("failed to open meta: {error}")))?;
    let rows: Vec<RemoteConcept> = meta
        .query()
        .select(Query::<RemoteConcept> {
            this: Term::var("this"),
            name: Term::var("name"),
            origin: Term::from(remote_dom::Origin::from(replica.this().clone())),
            subject: Term::var("subject"),
            address: Term::var("address"),
        })
        .perform(operator)
        .try_vec()
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to query remote records: {error:?}"))
        })?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let subject = row.subject.0.to_string().parse::<Did>().ok()?;
            let address = row.address.decode().ok()?;
            Some((row.name.0, RecordedRemote { subject, address }))
        })
        .collect())
}

/// The remote `repository` names `name`, if it records one.
pub(crate) async fn find<R: Principal>(
    repository: &Repository<R>,
    name: &str,
    operator: &DefaultOperator,
) -> Result<Option<RecordedRemote>, TonkWorkerError> {
    Ok(list(repository, operator)
        .await?
        .into_iter()
        .find_map(|(recorded, remote)| (recorded == name).then_some(remote)))
}

/// Connect to the remote `repository` names `name`.
pub(crate) async fn load<R: Principal>(
    repository: &Repository<R>,
    name: &str,
    operator: &DefaultOperator,
) -> Result<ConnectedReplica, TonkWorkerError> {
    find(repository, name, operator)
        .await?
        .ok_or_else(|| TonkWorkerError::NotFound(format!("no remote named '{name}'")))?
        .connect(operator)
        .await
}

/// Whether `branch` of the repository `repo` pulls from the branch
/// `remote_branch` of the remote the repository names `remote`.
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn tracks(
    tonk: &crate::worker::TonkState,
    repo: &str,
    branch: &dialog_repository::Branch,
    remote: &str,
    remote_branch: &str,
) -> bool {
    use dialog_repository::{RepositoryExt as _, Upstream};

    let Ok(repository) = tonk
        .profile
        .space(repo)
        .load()
        .perform(&tonk.operator)
        .await
    else {
        return false;
    };
    let Ok(Some(recorded)) = find(&repository, remote, &tonk.operator).await else {
        return false;
    };
    branch.pulls().iter().any(|upstream| {
        matches!(upstream, Upstream::Remote { remote, branch, .. }
            if recorded.is(remote) && branch == remote_branch)
    })
}
