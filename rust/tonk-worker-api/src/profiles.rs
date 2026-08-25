//! Profile roster and switching wire DTOs.

use serde::{Deserialize, Serialize};

/// One profile signed in (or local) on this browser, as the switcher
/// renders it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileRosterEntry {
    /// Storage name the profile opens under — the activation handle.
    pub profile_name: String,
    /// Account root the profile is attached to. Absent for a local
    /// workspace (never signed in, or signed out).
    pub root_did: Option<String>,
    /// Attached provider base URL.
    pub provider: Option<String>,
    /// Account email, captured best-effort at link time. May lag.
    pub email: Option<String>,
    /// Display name at last refresh.
    pub display_name: Option<String>,
    /// Whether this entry is the profile currently being served.
    pub active: bool,
}

/// Response body for `GET /api/profiles` and the switching routes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfilesResponse {
    /// Name of the profile currently being served.
    pub active: String,
    /// Every profile this browser knows, active entry included.
    pub profiles: Vec<ProfileRosterEntry>,
}

/// Request body for `POST /api/profiles/activate`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivateProfileRequest {
    /// Name of the roster profile to swap in.
    pub profile: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_serializes_roster_entries_in_camel_case() {
        let json = serde_json::to_value(ProfileRosterEntry {
            profile_name: "tonk".into(),
            root_did: Some("did:key:root".into()),
            provider: Some("https://accounts.example".into()),
            email: Some("person@example.com".into()),
            display_name: Some("Alice".into()),
            active: true,
        })
        .unwrap();
        assert_eq!(json["profileName"], "tonk");
        assert_eq!(json["rootDid"], "did:key:root");
        assert_eq!(json["displayName"], "Alice");
        assert_eq!(json["active"], true);
        assert!(json.get("profile_name").is_none());
    }

    #[dialog_common::test]
    fn it_round_trips_a_profiles_response() {
        let response = ProfilesResponse {
            active: "tonk".into(),
            profiles: vec![ProfileRosterEntry {
                profile_name: "tonk-0a".into(),
                root_did: None,
                provider: None,
                email: None,
                display_name: None,
                active: false,
            }],
        };
        let json = serde_json::to_string(&response).unwrap();
        assert_eq!(
            serde_json::from_str::<ProfilesResponse>(&json).unwrap(),
            response
        );
    }

    #[dialog_common::test]
    fn it_deserializes_an_activation_request() {
        let request: ActivateProfileRequest =
            serde_json::from_str(r#"{"profile":"tonk-0a"}"#).unwrap();
        assert_eq!(request.profile, "tonk-0a");
    }
}
