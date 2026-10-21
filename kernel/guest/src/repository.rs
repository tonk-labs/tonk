//! A repository of Dialog DB on one layer, signed for by one key. Trimmed
//! from `sapling-dialog`'s repository: no stack, one subject.

use dialog_artifacts::selector::Constrained;
use dialog_artifacts::{Artifact, ArtifactSelector, Changes, Instruction};

use dialog_credentials::SignerCredential;
use dialog_peer::{Peer, Session};
use dialog_repository::{
    Branch as DialogBranch, CommitError, PublishError, Repository as DialogRepository, Revision,
};
use dialog_storage::provider::storage::Storage;
use dialog_storage::provider::{Space, Volatile};
use dialog_varsig::Did;
use futures_util::StreamExt as _;

use crate::layer::{LayerError, SharedLayer};
use crate::mount::Mount;
use crate::store::{Store, Witness};

/// The space of Dialog DB over a layer: the archive, the memory, and the blobs
/// in the layer, and the credentials and certificates in memory.
pub type LayerSpace = Space<Store, Store, Volatile, Volatile, Store>;

/// The branch of the subject that the peer keeps its records in.
const STATE: &str = "main";

/// What a call on a [`Repository`] did not do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The layer, or its chain, does not allow this.
    Denied(String),
    /// A compare and set lost a race: another writer moved the branch. Open
    /// the branch again, and retry.
    Conflict,
    /// The chain of the layer expired. Open the layer again.
    Expired,
    /// The input is not valid.
    Invalid(String),
    /// A change writes to an attribute that Dialog DB keeps for itself.
    Reserved(String),
    /// The layer did not answer.
    Unavailable(String),
    /// No route to a peer answered: a failure of the network.
    Unreachable(String),
    /// An exchange with a peer broke off, or got no answer in time: a
    /// failure of the network.
    BrokeOff(String),
    /// The blob store has no room for the bytes, as the quota of the device
    /// says.
    Full(String),
    /// Dialog DB failed, for a reason of its own.
    Dialog(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Denied(reason) => write!(f, "denied: {reason}"),
            Self::Conflict => f.write_str("conflict"),
            Self::Expired => f.write_str("expired"),
            Self::Invalid(reason) => write!(f, "invalid: {reason}"),
            Self::Unavailable(reason) => write!(f, "unavailable: {reason}"),
            Self::Unreachable(reason) => write!(f, "unreachable: {reason}"),
            Self::BrokeOff(reason) => write!(f, "broke off: {reason}"),
            Self::Full(reason) => write!(f, "full: {reason}"),
            Self::Reserved(attribute) => write!(f, "the attribute {attribute} is reserved"),
            Self::Dialog(reason) => write!(f, "{reason}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<LayerError> for Error {
    fn from(error: LayerError) -> Self {
        match error {
            LayerError::Denied(reason) => Self::Denied(reason),
            LayerError::Conflict => Self::Conflict,
            LayerError::Expired => Self::Expired,
            LayerError::Invalid(reason) => Self::Invalid(reason),
            LayerError::Unavailable(reason) => Self::Unavailable(reason),
            LayerError::Unreachable(reason) => Self::Unreachable(reason),
            LayerError::BrokeOff(reason) => Self::BrokeOff(reason),
            LayerError::Full(reason) => Self::Full(reason),
        }
    }
}

/// A repository of Dialog DB on one layer.
#[derive(Clone)]
pub struct Repository {
    peer: Peer<LayerSpace, Session>,
    subject: Did,
    witness: Witness,
}

impl Repository {
    /// Opens the repository of the subject of `layer`, whose commits `signer`
    /// signs. Each revision names the subject as its authority: Dialog's
    /// `main` has no builder method yet that names another (Sapling's
    /// branch calls it `profile`).
    pub async fn open(layer: SharedLayer, signer: SignerCredential) -> Result<Self, Error> {
        let subject = layer.subject();
        let witness = Witness::default();
        let mount = Mount::new(Store::new(layer.clone(), witness.clone()));
        let builder = Peer::operator(signer);
        let storage = Storage::<LayerSpace>::new().owned_by(subject.clone());
        let peer = builder
            .space(DialogRepository::from(subject.clone()).branch(STATE))
            .at(mount.location().clone())
            .with(storage)
            .build()
            .await;
        drop(mount);
        let peer = peer.map_err(|error| witnessed(&witness, &error))?;
        Ok(Self {
            peer,
            subject,
            witness,
        })
    }

    /// Opens the branch `name`, at its head now.
    pub async fn branch(&self, name: &str) -> Result<Branch, Error> {
        self.witness.clear();
        let branch = DialogRepository::from(self.subject.clone())
            .branch(name)
            .open()
            .perform(&self.peer)
            .await
            .map_err(|error| witnessed(&self.witness, &error))?;
        Ok(Branch {
            repository: self.clone(),
            branch,
        })
    }
}

/// A branch of a [`Repository`], at the head it was opened at, or that its
/// last commit made.
pub struct Branch {
    repository: Repository,
    branch: DialogBranch,
}

impl Branch {
    /// Commits `instructions` on the branch, over the head it holds.
    pub async fn commit(&self, instructions: Vec<Instruction>) -> Result<Option<Revision>, Error> {
        let witness = &self.repository.witness;
        witness.clear();
        let changes: Changes = instructions.into_iter().collect();
        self.branch
            .transaction()
            .integrate(changes)
            .commit()
            .publish()
            .perform(&self.repository.peer)
            .await
            .map_err(|error| match error {
                CommitError::Publish(PublishError::VersionMismatch { .. }) => Error::Conflict,
                CommitError::Artifact(
                    dialog_artifacts::DialogArtifactsError::ReservedRelation(attribute),
                ) => Error::Reserved(attribute),
                error => witnessed(witness, &error),
            })
            .map(Some)
    }

    /// The artifacts of the branch that `selector` selects.
    pub async fn select(
        &self,
        selector: ArtifactSelector<Constrained>,
    ) -> Result<Vec<Artifact>, Error> {
        let witness = &self.repository.witness;
        witness.clear();
        let failed = |error: &dyn std::fmt::Display| witnessed(witness, error);
        let views = self
            .branch
            .claims()
            .select(selector)
            .to_owned()
            .perform(&self.repository.peer)
            .await
            .map_err(|error| failed(&error))?
            .collect::<Vec<_>>()
            .await;
        views
            .into_iter()
            .map(|artifact| artifact.map_err(|error| failed(&error)))
            .collect()
    }
}

/// The error of a call that Dialog DB failed with `error`: what the layer
/// said, if it said anything, since Dialog DB keeps some errors as text.
pub(crate) fn witnessed(witness: &Witness, error: &dyn std::fmt::Display) -> Error {
    match witness.last() {
        Some(said) => Error::from(said),
        None => Error::Dialog(error.to_string()),
    }
}
