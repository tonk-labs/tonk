//! UCAN attribute definitions using dialog-query's #[derive(Attribute)] macro.
//!
//! This module defines typed attributes for storing UCAN delegation facts in dialog-db.
//! Each attribute corresponds to a field in a UCAN delegation and is stored with
//! the `ucan/` namespace prefix.

use dialog_query::Attribute;

/// The issuer DID of a UCAN delegation (the entity granting the delegation).
#[derive(Attribute, Clone, PartialEq)]
pub struct Issuer(pub String);

/// The audience DID of a UCAN delegation (the entity receiving the delegation).
#[derive(Attribute, Clone, PartialEq)]
pub struct Audience(pub String);

/// The subject DID of a UCAN delegation (what the delegation applies to).
/// For powerline delegations, this is "*".
#[derive(Attribute, Clone, PartialEq)]
pub struct Subject(pub String);

/// The command path being delegated (e.g., "/read/write").
#[derive(Attribute, Clone, PartialEq)]
pub struct Cmd(pub String);

/// Raw DAG-CBOR bytes of the delegation (stored as blob).
#[derive(Attribute, Clone, PartialEq)]
pub struct Blob(pub Vec<u8>);
