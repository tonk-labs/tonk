pub mod delegation;
pub mod invite;
pub mod operator;
pub mod ownership;
pub mod schema;
pub mod secret;
pub mod space;

pub use delegation::{Delegation, DelegationError};
pub use invite::{
    InviteEnvelopeV1, InviteError, InviteGrantV1, create_invite, create_space_grant,
    decode_invite, encode_invite, verify_envelope, verify_grant,
};
pub use operator::Operator;
pub use secret::*;

// Re-export UCAN types for delegation creation
pub use ::dialog_credentials::Ed25519Signer;
pub use ::dialog_ucan::Delegation as UcanDelegation;
pub use ::dialog_ucan::command::Command;
pub use ::dialog_ucan::subject::Subject;
pub use ::dialog_ucan::time::Timestamp;
pub use ::dialog_varsig::Did;
pub use ::dialog_varsig::eddsa::Ed25519Signature;
pub use dialog_artifacts::PlatformBackend;
pub use dialog_query::claim::Transaction;
pub use dialog_query::{ArtifactAttribute as Attribute, Entity, Fact, Relation, Value};
pub use ownership::Ownership;
pub use space::{
    BranchInfo, CredentialsInfo, MemoryBackend, MemoryStorageBackend, RemoteBranchInfo,
    RemoteState, Revision, SiteInfo, Space, SpaceError, UpstreamInfo,
};

#[cfg(not(target_arch = "wasm32"))]
pub use space::{FileSystemStorageBackend, FsBackend};
