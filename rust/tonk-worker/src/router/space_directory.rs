//! What the person's profile keeps of a space it does not hold.
//!
//! Where sites have origins of their own, a space's content and its replica
//! are held by the worker on the space's origin (see
//! [`space_worker`](super::space_worker)). The worker holding the person's
//! profile mounts nothing for a space: it holds the delegations it acts on
//! the space with, in its own access branch, and what it knows of the space
//! in the account directory on its own `main`: that the person has it, what
//! it is called, and where it syncs. This is how it reads that.

use dialog_credentials::{Credential, Verifier};
use dialog_repository::{Repository, SiteAddress};
use dialog_varsig::Did;
use tonk_schema::prelude::DidExt as _;
use url::Url;

use super::create_invite::{
    ConfiguredRemoteExecutionUrls, ConfiguredRemoteRequirement, RemoteRefusal,
};
use super::repository::{CONTENT_BRANCH, RepositoryConfiguration, RepositoryInfo};
use crate::{TonkWorkerError, worker::TonkState};

/// `subject` as a repository this worker names and does not mount: its
/// public identity, with nothing opened in storage. What is done to it is
/// proven with the delegations the profile holds.
pub(crate) fn handle(subject: &Did) -> Result<Repository<Credential>, TonkWorkerError> {
    let verifier: Verifier = subject
        .to_string()
        .parse()
        .map_err(|_| TonkWorkerError::Router(format!("{subject} names no public key")))?;
    Ok(Repository::from(Credential::from(verifier)))
}

/// The space `key` names, when the person has it: listed in the account
/// directory, or recorded as a replica of this profile's on any of its
/// branches. Named and not mounted (see [`handle`]).
pub(crate) async fn held(
    tonk: &TonkState,
    key: &str,
) -> Result<Repository<Credential>, TonkWorkerError> {
    let missing = || TonkWorkerError::NotFound(format!("the person has no space '{key}'"));
    let subject = super::adopt::space_subject(key).ok_or_else(missing)?;
    if super::join::find_replica_for_subject(tonk, &subject).await?
        || configuration(tonk, &subject).await?.is_some()
    {
        return handle(&subject);
    }
    // A space made or joined on another of this profile's branches (a
    // workspace it was signed out on) is still the person's while a sign-in
    // carries it over. Whether this branch's authority reaches it is for
    // whatever is then asked of it to prove.
    for (branch, _) in super::profile::local_branches(tonk).await {
        if branch != tonk.active_branch
            && super::profile_name::real_space_keys_on(tonk, &branch)
                .await
                .iter()
                .any(|listed| listed.as_str() == subject.as_str())
        {
            return handle(&subject);
        }
    }
    Err(missing())
}

/// `requested` laid over `existing` the way attaching a remote to a mounted
/// space does: a remote already named is kept as it is, and a branch that
/// already has an upstream keeps it. What comes back is what the space is
/// to sync with, for the directory to record.
pub(crate) fn merged(
    mut existing: RepositoryConfiguration,
    requested: &RepositoryConfiguration,
) -> RepositoryConfiguration {
    for (name, remote) in &requested.remote {
        existing
            .remote
            .entry(name.clone())
            .or_insert_with(|| remote.clone());
    }
    for (name, branch) in &requested.branch {
        let kept = existing.branch.entry(name.clone()).or_default();
        if kept.upstream.is_none() {
            kept.upstream = branch.upstream.clone();
        }
    }
    existing
}

/// Where `subject` syncs and which branches it has: as the account
/// directory records it, or, for a space being joined, as its invitation
/// says ([`expect`]). `None` for a space that is neither.
pub(crate) async fn configuration(
    tonk: &TonkState,
    subject: &Did,
) -> Result<Option<RepositoryConfiguration>, TonkWorkerError> {
    if let Some(listed) = super::adopt::directory_configuration_strict(tonk, subject).await? {
        return Ok(Some(listed));
    }
    let bytes = match tonk
        .profile
        .secrets()
        .site(expected_site(subject))
        .load::<Vec<u8>>()
        .perform(&tonk.profile)
        .await
    {
        Ok(bytes) => bytes,
        Err(error) if crate::credential::is_missing(&error) => return Ok(None),
        Err(error) => {
            return Err(TonkWorkerError::Internal(format!(
                "failed to read where {subject} is expected to sync: {error}"
            )));
        }
    };
    // Settled: see [`settle`].
    if bytes.is_empty() {
        return Ok(None);
    }
    serde_json::from_slice(&bytes).map(Some).map_err(|error| {
        TonkWorkerError::Internal(format!(
            "where {subject} is expected to sync is illegible: {error}"
        ))
    })
}

/// The profile secret a space being joined is expected under.
const EXPECTED_SITE_PREFIX: &str = "tonk-space-expected:";

fn expected_site(subject: &Did) -> String {
    format!("{EXPECTED_SITE_PREFIX}{subject}")
}

/// Expect `subject`, a space being joined, to sync as `configuration` says.
///
/// The space's own worker is delegated to and told where the space syncs
/// before the claim it is asked to commit has landed, and the directory
/// lists a space only once it has. Kept beside the profile, like a new
/// space's seed, so a join a restart interrupted finds it. [`settle`]d when
/// the join ends, either way.
pub(crate) async fn expect(
    tonk: &TonkState,
    subject: &Did,
    configuration: &RepositoryConfiguration,
) -> Result<(), TonkWorkerError> {
    let bytes = serde_json::to_vec(configuration)
        .map_err(|error| TonkWorkerError::Internal(format!("configuration: {error}")))?;
    save_expected(tonk, subject, bytes).await
}

/// `subject` is no longer [expected](expect): its join landed, and the
/// directory lists it, or it failed. Best effort: what is left behind names
/// a space the profile holds no authority to act on.
pub(crate) async fn settle(tonk: &TonkState, subject: &Did) {
    if let Err(error) = save_expected(tonk, subject, Vec::new()).await {
        tonk_common::log!("{subject} is still expected: {error}");
    }
}

async fn save_expected(
    tonk: &TonkState,
    subject: &Did,
    bytes: Vec<u8>,
) -> Result<(), TonkWorkerError> {
    tonk.profile
        .secrets()
        .site(expected_site(subject))
        .save(bytes)
        .perform(&tonk.profile)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!(
                "failed to keep where {subject} is expected to sync: {error}"
            ))
        })
}

/// The UCAN endpoint `configuration` has the space's content branch sync
/// with, or why there is none an invitation could name.
pub(crate) fn endpoint(
    configuration: &RepositoryConfiguration,
) -> Result<ConfiguredRemoteRequirement, TonkWorkerError> {
    let Some(upstream) = configuration
        .branch
        .get(CONTENT_BRANCH)
        .and_then(|branch| branch.upstream.as_ref())
    else {
        return Ok(ConfiguredRemoteRequirement::Refused(
            RemoteRefusal::NotSynced,
        ));
    };
    let Some(remote) = configuration.remote.get(&upstream.remote) else {
        return Ok(ConfiguredRemoteRequirement::Refused(
            RemoteRefusal::NotSynced,
        ));
    };
    let SiteAddress::Ucan(ucan) = &remote.address else {
        return Ok(ConfiguredRemoteRequirement::Refused(
            RemoteRefusal::UnshareableRemote,
        ));
    };
    let access_url = Url::parse(ucan.endpoint()).map_err(|e| {
        TonkWorkerError::Internal(format!(
            "remote '{}' has unparseable UCAN endpoint '{}': {e}",
            upstream.remote,
            ucan.endpoint()
        ))
    })?;
    Ok(ConfiguredRemoteRequirement::Ready(
        ConfiguredRemoteExecutionUrls { access_url },
    ))
}

/// What the profile can say of `subject` without asking the worker that
/// holds it: its identity, its name, and where it syncs, which is nowhere
/// for a space the directory names no remote for. No revisions and no
/// members, which only that worker has.
pub(crate) async fn info(
    tonk: &TonkState,
    subject: &Did,
) -> Result<RepositoryInfo, TonkWorkerError> {
    let configuration = configuration(tonk, subject).await?.unwrap_or_default();
    let key = subject.repo_key();
    let label = super::repository::directory_space_name(tonk, subject)
        .await
        .unwrap_or_else(|| key.to_owned());
    Ok(RepositoryInfo {
        name: key.to_owned(),
        label,
        subject: subject.clone(),
        operator: tonk.operator.did(),
        profile: tonk.profile.did(),
        branch: configuration.branch,
        remote: configuration.remote,
        members: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use dialog_remote_ucan::UcanAddress;
    use dialog_repository::SiteAddress;

    use super::{endpoint, merged};
    use crate::router::create_invite::{ConfiguredRemoteRequirement, RemoteRefusal};
    use crate::router::repository::{
        BranchConfiguration, RemoteConfiguration, RepositoryConfiguration,
    };

    fn synced(with: &str) -> RepositoryConfiguration {
        RepositoryConfiguration::default()
            .remote(
                "origin",
                RemoteConfiguration::new(SiteAddress::from(UcanAddress::new(with))),
            )
            .branch(
                "main",
                BranchConfiguration::default().upstream("origin", "main"),
            )
    }

    fn local() -> RepositoryConfiguration {
        RepositoryConfiguration::default().branch("main", BranchConfiguration::default())
    }

    #[dialog_common::test]
    fn it_reads_the_endpoint_the_content_branch_syncs_with() {
        let ready = endpoint(&synced("https://sync.example.test/ucan/")).unwrap();

        assert!(matches!(
            ready,
            ConfiguredRemoteRequirement::Ready(remote)
                if remote.access_url.as_str() == "https://sync.example.test/ucan/"
        ));
    }

    #[dialog_common::test]
    fn it_says_a_space_with_no_upstream_is_not_synced() {
        for configuration in [local(), RepositoryConfiguration::default()] {
            assert!(matches!(
                endpoint(&configuration).unwrap(),
                ConfiguredRemoteRequirement::Refused(RemoteRefusal::NotSynced)
            ));
        }
    }

    #[dialog_common::test]
    fn it_attaches_a_remote_to_a_space_that_has_none() {
        let attached = merged(local(), &synced("https://sync.example.test/ucan/"));

        assert_eq!(attached.remote.keys().collect::<Vec<_>>(), ["origin"]);
        let upstream = attached.branch["main"].upstream.as_ref().unwrap();
        assert_eq!(
            (upstream.remote.as_str(), upstream.branch.as_str()),
            ("origin", "main")
        );
    }

    #[dialog_common::test]
    fn it_keeps_the_remote_a_space_already_syncs_with() {
        let kept = merged(
            synced("https://first.example.test/ucan/"),
            &synced("https://second.example.test/ucan/"),
        );

        assert!(matches!(
            endpoint(&kept).unwrap(),
            ConfiguredRemoteRequirement::Ready(remote)
                if remote.access_url.as_str() == "https://first.example.test/ucan/"
        ));
    }
}
