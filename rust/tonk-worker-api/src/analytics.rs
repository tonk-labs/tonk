//! Worker-to-page notifications for successful launch-funnel operations.
//!
//! These messages stay within the browser. The page-side analytics boundary
//! hashes the local space routing key before capture.

use serde::{Deserialize, Serialize};

/// Fixed worker-message discriminator for [`AnalyticsMessage`].
pub const ANALYTICS_MESSAGE: &str = "tonk-analytics";

/// One successful lifecycle operation worth recording in the launch funnel.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "name", rename_all = "snake_case")]
pub enum AnalyticsEvent {
    /// A new local space completed creation.
    SpaceCreated {
        /// Local routing key, hashed by the page before remote capture.
        space: String,
        /// Validated catalog reference; hashed by the page before capture.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        template: Option<String>,
    },
    /// A new local replica completed an invite join.
    SpaceJoined {
        /// Local routing key, hashed by the page before remote capture.
        space: String,
    },
    /// A share invite was durably minted for a space.
    SpaceShared {
        /// Local routing key, hashed by the page before remote capture.
        space: String,
    },
    /// The profile is linked to its account without the account's
    /// encryption key: by signing in through another deployment, or by a
    /// link made before that key existed.
    AccountWithoutKey,
    /// A step of an account attempt the worker ran: adding an account,
    /// logging in, confirming an address. The page validates the event
    /// before capture, as it does one of its own.
    Account {
        /// The step, in the account journey's closed vocabulary.
        event: tonk_analytics::account::AccountEvent,
    },
}

/// Typed message posted from the service worker to its originating page.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct AnalyticsMessage {
    /// Fixed [`ANALYTICS_MESSAGE`] discriminator.
    #[serde(rename = "type")]
    pub message_type: String,
    /// Successful operation and its local space key.
    #[serde(flatten)]
    pub event: AnalyticsEvent,
}

impl AnalyticsMessage {
    /// Wrap a successful lifecycle operation for delivery to the page.
    pub fn new(event: AnalyticsEvent) -> Self {
        Self {
            message_type: ANALYTICS_MESSAGE.to_owned(),
            event,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creation_messages_support_old_workers_and_template_attribution() {
        let legacy = serde_json::json!({
            "type": "tonk-analytics", "name": "space_created", "space": "abc"
        });
        let message: AnalyticsMessage = serde_json::from_value(legacy).unwrap();
        assert_eq!(
            message.event,
            AnalyticsEvent::SpaceCreated {
                space: "abc".into(),
                template: None,
            }
        );
        let attributed = AnalyticsMessage::new(AnalyticsEvent::SpaceCreated {
            space: "abc".into(),
            template: Some("https://example.com/catalog.json#notes".into()),
        });
        let roundtrip: AnalyticsMessage =
            serde_json::from_value(serde_json::to_value(&attributed).unwrap()).unwrap();
        assert_eq!(roundtrip, attributed);
    }

    #[test]
    fn analytics_message_has_a_closed_wire_shape() {
        let message = AnalyticsMessage::new(AnalyticsEvent::SpaceCreated {
            space: "abc".to_owned(),
            template: None,
        });
        assert_eq!(
            serde_json::to_value(&message).unwrap(),
            serde_json::json!({
                "type": "tonk-analytics",
                "name": "space_created",
                "space": "abc",
            })
        );
    }
}
