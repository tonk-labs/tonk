//! Closed launch-funnel analytics schema.
//!
//! Attribution inputs are interpreted locally and reduced to reviewed enum
//! values before capture. Raw URLs, referrers, DIDs, UCANs, and delegation
//! bytes never enter an event payload.

use serde::Serialize;
use serde_json::{Value, json};
use url::Url;

/// Canonical query parameter for Tonk campaign links.
pub const CHANNEL_PARAMETER: &str = "tonk_channel";

/// One of the three acquisition channels in the launch funnel.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    /// A link Tonk sent directly to a known person or group.
    WarmOutreach,
    /// A shared space link that moved organically between people.
    OrganicReshare,
    /// Discovery through the public web or Tonk's network shell.
    ClearnetDiscovery,
}

/// Evidence used to choose [`Channel`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributionSource {
    /// The canonical [`CHANNEL_PARAMETER`] was present.
    UrlParameter,
    /// A reviewed UTM value selected the channel.
    Utm,
    /// An external referrer selected the channel class.
    Referrer,
    /// The route type supplied the safe fallback.
    Inferred,
}

/// Surface on which the browser session entered Tonk.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryType {
    /// The Tonk network shell, including the landing page and Hub.
    TonkNetwork,
    /// A space or invite/join route.
    SharedSpace,
}

/// Content-free session attribution registered before any funnel event.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Attribution {
    /// Acquisition channel.
    pub channel: Channel,
    /// Evidence used to classify the channel.
    pub attribution_source: AttributionSource,
    /// Network-shell versus shared-space entry.
    pub entry_type: EntryType,
    /// Normalized entry path. Dynamic segments are hashed locally.
    pub entry_route: String,
    /// Hashed space route key, when the landing path exposes one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_space_id: Option<String>,
}

impl Attribution {
    /// Classify a landing URL and optional document referrer.
    ///
    /// `tonk_channel` wins over UTM values. When neither is present, an
    /// external referrer is recorded only as a class (never a domain), and
    /// the entry surface supplies the channel: shared spaces are organic
    /// re-shares; the Tonk network shell is clearnet discovery.
    pub fn from_urls(landing_url: &str, referrer: Option<&str>) -> Option<Self> {
        let landing = Url::parse(landing_url).ok()?;
        let entry_type = entry_type(landing.path());
        let explicit = landing
            .query_pairs()
            .find(|(key, _)| key == CHANNEL_PARAMETER)
            .and_then(|(_, value)| canonical_channel(&value))
            .map(|channel| (channel, AttributionSource::UrlParameter));
        let utm = landing
            .query_pairs()
            .filter(|(key, _)| matches!(key.as_ref(), "utm_source" | "utm_medium" | "utm_campaign"))
            .find_map(|(_, value)| utm_channel(&value))
            .map(|channel| (channel, AttributionSource::Utm));
        let external_referrer = referrer
            .filter(|value| !value.is_empty())
            .and_then(|value| Url::parse(value).ok())
            .is_some_and(|source| source.origin() != landing.origin());
        let inferred = inferred_channel(entry_type);
        let (channel, attribution_source) = explicit.or(utm).unwrap_or({
            (
                inferred,
                if external_referrer {
                    AttributionSource::Referrer
                } else {
                    AttributionSource::Inferred
                },
            )
        });

        Some(Self {
            channel,
            attribution_source,
            entry_type,
            entry_route: crate::normalize_path(landing.path()),
            entry_space_id: space_key(landing.path()).map(crate::anonymize),
        })
    }

    /// Serialize the reviewed attribution fields as PostHog properties.
    pub fn properties(&self) -> Value {
        serde_json::to_value(self).expect("Attribution has an infallible JSON shape")
    }
}

/// Successful space conversion kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpaceConversion {
    /// A new local space was successfully created.
    Created,
    /// A new local replica was successfully joined from an invite.
    Joined,
}

/// Properties for one successful create/join conversion.
pub fn space_conversion_properties(conversion: SpaceConversion, space_key: &str) -> Value {
    json!({
        "schema_version": 1,
        "conversion": conversion,
        "space_id": crate::anonymize(space_key),
    })
}

/// Properties for an invite successfully minted for a space.
pub fn space_shared_properties(space_key: &str) -> Value {
    json!({
        "schema_version": 1,
        "space_id": crate::anonymize(space_key),
    })
}

/// Visit properties combine schema version with session attribution.
pub fn visit_properties(attribution: &Attribution) -> Value {
    let Value::Object(mut properties) = attribution.properties() else {
        unreachable!("Attribution serializes as an object")
    };
    properties.insert("schema_version".to_owned(), Value::from(1));
    Value::Object(properties)
}

/// Properties for a successfully created account.
pub fn account_created_properties() -> Value {
    json!({ "schema_version": 1 })
}

fn entry_type(path: &str) -> EntryType {
    let first = path.split('/').find(|segment| !segment.is_empty());
    if matches!(first, Some("space" | "join")) {
        EntryType::SharedSpace
    } else {
        EntryType::TonkNetwork
    }
}

fn inferred_channel(entry_type: EntryType) -> Channel {
    match entry_type {
        EntryType::SharedSpace => Channel::OrganicReshare,
        EntryType::TonkNetwork => Channel::ClearnetDiscovery,
    }
}

fn canonical_channel(value: &str) -> Option<Channel> {
    match normalized(value).as_str() {
        "outreach" | "warm" | "warm_outreach" => Some(Channel::WarmOutreach),
        "organic" | "reshare" | "organic_reshare" => Some(Channel::OrganicReshare),
        "clearnet" | "discovery" | "clearnet_discovery" => Some(Channel::ClearnetDiscovery),
        _ => None,
    }
}

fn utm_channel(value: &str) -> Option<Channel> {
    match normalized(value).as_str() {
        "outreach" | "warm" | "warm_outreach" | "email" => Some(Channel::WarmOutreach),
        "reshare" | "organic_reshare" | "share" => Some(Channel::OrganicReshare),
        "clearnet" | "discovery" | "clearnet_discovery" | "organic" | "search" => {
            Some(Channel::ClearnetDiscovery)
        }
        _ => None,
    }
}

fn normalized(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace(['-', ' '], "_")
}

fn space_key(path: &str) -> Option<&str> {
    let mut segments = path.split('/').filter(|segment| !segment.is_empty());
    (segments.next() == Some("space"))
        .then(|| segments.next())
        .flatten()
        .filter(|segment| !segment.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn explicit_channel_wins_and_raw_attribution_never_leaves() {
        let raw_space = "did:key:z6MkSecretSpace";
        let attribution = Attribution::from_urls(
            &format!(
                "https://tonk.network/space/{raw_space}/view/PrivateNotes?tonk_channel=outreach&utm_source=organic"
            ),
            Some("https://mail.example/private/thread"),
        )
        .expect("valid landing URL");

        assert_eq!(attribution.channel, Channel::WarmOutreach);
        assert_eq!(
            attribution.attribution_source,
            AttributionSource::UrlParameter
        );
        assert_eq!(attribution.entry_type, EntryType::SharedSpace);
        assert_eq!(
            attribution.entry_space_id.as_deref(),
            Some(crate::anonymize(raw_space).as_str())
        );
        let payload = visit_properties(&attribution).to_string();
        for sentinel in [
            raw_space,
            "PrivateNotes",
            "mail.example",
            "private/thread",
            "tonk_channel",
            "utm_source",
        ] {
            assert!(!payload.contains(sentinel), "payload leaked {sentinel}");
        }
    }

    #[dialog_common::test]
    fn utm_values_map_to_the_reviewed_channel_vocabulary() {
        let outreach = Attribution::from_urls("https://tonk.network/?utm_medium=email", None)
            .expect("valid URL");
        let reshare = Attribution::from_urls(
            "https://tonk.network/join?utm_campaign=organic-reshare",
            None,
        )
        .expect("valid URL");
        let discovery = Attribution::from_urls("https://tonk.network/?utm_medium=organic", None)
            .expect("valid URL");

        assert_eq!(outreach.channel, Channel::WarmOutreach);
        assert_eq!(reshare.channel, Channel::OrganicReshare);
        assert_eq!(discovery.channel, Channel::ClearnetDiscovery);
        assert_eq!(outreach.attribution_source, AttributionSource::Utm);
    }

    #[dialog_common::test]
    fn route_type_supplies_safe_fallback_attribution() {
        let network = Attribution::from_urls("https://tonk.network/", None).unwrap();
        let shared = Attribution::from_urls(
            "https://tonk.network/join?access=secret#private",
            Some("https://chat.example/thread/secret"),
        )
        .unwrap();

        assert_eq!(network.channel, Channel::ClearnetDiscovery);
        assert_eq!(network.attribution_source, AttributionSource::Inferred);
        assert_eq!(shared.channel, Channel::OrganicReshare);
        assert_eq!(shared.attribution_source, AttributionSource::Referrer);
        assert_eq!(shared.entry_route, "/join");
        assert_eq!(shared.entry_space_id, None);
        let payload = visit_properties(&shared).to_string();
        assert!(!payload.contains("access"));
        assert!(!payload.contains("private"));
        assert!(!payload.contains("chat.example"));
    }

    #[dialog_common::test]
    fn conversion_properties_hash_the_space_key() {
        let raw = "z6MkSecretSpace";
        let conversion = space_conversion_properties(SpaceConversion::Joined, raw);
        let shared = space_shared_properties(raw);

        assert_eq!(conversion["conversion"], "joined");
        assert_eq!(conversion["space_id"], crate::anonymize(raw));
        assert_eq!(shared["space_id"], crate::anonymize(raw));
        assert!(!conversion.to_string().contains(raw));
        assert!(!shared.to_string().contains(raw));
    }
}
