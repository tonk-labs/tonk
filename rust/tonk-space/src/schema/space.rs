//! Space-specific attribute definitions.
//!
//! This module defines typed attributes for space ownership and management.

use dialog_query::{Attribute, Entity};

/// Links a space DID to the ownership delegation CID.
#[derive(Attribute, Clone, PartialEq)]
pub struct Owner(pub Entity);
