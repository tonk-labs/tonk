//! UCAN verification module

pub mod capability;
pub mod verify;

pub use capability::{BlobAllocate, BlobGet, Capability};
pub use verify::{VerificationError, verify_invocation};
