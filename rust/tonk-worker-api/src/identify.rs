//! Identity-route wire DTO.

use serde::{Deserialize, Serialize};

/// Response containing the user's DID.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdentifyResponse {
    /// The user's decentralized identifier (DID).
    pub did: String,
}
