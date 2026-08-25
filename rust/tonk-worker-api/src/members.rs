//! Space membership management wire DTOs: promoting a member to admin
//! and removing a member.

use dialog_varsig::Did;
use serde::{Deserialize, Serialize};

/// Request body for `POST /api/repository/{repo}/admins`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromoteMemberRequest {
    /// The member to promote: the DID their membership is keyed on, an
    /// account root for a linked profile.
    pub member: Did,
}

/// Response body for `POST /api/repository/{repo}/admins`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromoteMemberAcknowledgement {
    /// The member promoted.
    pub member: Did,
    /// CID of the leaf of the admin chain minted for them: the hop a
    /// demotion revokes.
    pub target_cid: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_serializes_a_promotion_in_camel_case() {
        let did: Did = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"
            .parse()
            .unwrap();
        let value = serde_json::to_value(PromoteMemberAcknowledgement {
            member: did.clone(),
            target_cid: "bafycid".into(),
        })
        .unwrap();
        assert_eq!(value["targetCid"], "bafycid");
        let request: PromoteMemberRequest =
            serde_json::from_str(&format!(r#"{{"member":"{did}"}}"#)).unwrap();
        assert_eq!(request.member, did);
    }
}
