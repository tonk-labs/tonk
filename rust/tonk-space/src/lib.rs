pub mod delegation;
pub mod operator;
pub mod ownership;
mod relation;
pub mod secret;
pub mod space;

pub use delegation::{Delegation, DelegationError};
pub use operator::Operator;
pub use secret::*;

// Re-export UCAN types for delegation creation
pub use dialog_artifacts::PlatformBackend;
pub use dialog_query::claim::Transaction;
pub use ownership::Ownership;
pub use space::{
    AuthMethod, MemoryBackend, MemoryStorageBackend, RemoteState, RestStorageConfig, Revision,
    S3Authority, Space, SpaceError,
};
pub use ucan::Delegation as UcanDelegation;
pub use ucan::command::Command;
pub use ucan::delegation::subject::DelegatedSubject;
pub use ucan::did::{Ed25519Did, Ed25519Signer};
pub use ucan::time::timestamp::Timestamp;

#[cfg(not(target_arch = "wasm32"))]
pub use space::{FileSystemStorageBackend, FsBackend};
