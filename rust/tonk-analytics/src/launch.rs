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

/// Canonical query parameter naming the public platform that carried a link.
pub const SOURCE_PARAMETER: &str = "tonk_source";

/// Canonical query parameter carrying a hashed referring-space token.
pub const SPACE_PARAMETER: &str = "tonk_space";

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

/// Public platform credited with delivering the browser session.
///
/// This is deliberately closed: raw referrer hosts and arbitrary UTM values
/// never enter an event payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourcePlatform {
    /// No external or explicit source was available.
    Direct,
    /// An email campaign or webmail surface.
    Email,
    /// A public search engine.
    Search,
    /// X, including legacy Twitter and t.co links.
    X,
    /// LinkedIn.
    Linkedin,
    /// Instagram.
    Instagram,
    /// Facebook.
    Facebook,
    /// Reddit.
    Reddit,
    /// Discord.
    Discord,
    /// Slack.
    Slack,
    /// Telegram.
    Telegram,
    /// WhatsApp.
    Whatsapp,
    /// Bluesky.
    Bluesky,
    /// Mastodon.
    Mastodon,
    /// GitHub.
    Github,
    /// Product Hunt.
    ProductHunt,
    /// Hacker News.
    HackerNews,
    /// An external source outside the reviewed vocabulary.
    Other,
}

/// Evidence used to select [`SourcePlatform`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceDetection {
    /// The canonical [`SOURCE_PARAMETER`] was present.
    UrlParameter,
    /// `utm_source` was present.
    Utm,
    /// A reviewed public referrer host matched.
    Referrer,
    /// No external source was available.
    Direct,
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
    /// Public platform credited with delivering the visit.
    pub source_platform: SourcePlatform,
    /// Evidence used to classify [`Self::source_platform`].
    pub source_detection: SourceDetection,
    /// Network-shell versus shared-space entry.
    pub entry_type: EntryType,
    /// Normalized entry path. Dynamic segments are hashed locally.
    pub entry_route: String,
    /// Hashed referring-space key, from a visible route or validated token.
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
        let external_referrer = referrer
            .filter(|value| !value.is_empty())
            .and_then(|value| Url::parse(value).ok())
            .filter(|source| source.origin() != landing.origin());
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
        let inferred = inferred_channel(entry_type);
        let (channel, attribution_source) = explicit.or(utm).unwrap_or({
            (
                inferred,
                if external_referrer.is_some() {
                    AttributionSource::Referrer
                } else {
                    AttributionSource::Inferred
                },
            )
        });
        let (source_platform, source_detection) =
            source_attribution(&landing, external_referrer.as_ref().and_then(Url::host_str));
        let entry_space_id = space_key(landing.path())
            .map(crate::anonymize)
            .or_else(|| tagged_space(&landing));

        Some(Self {
            channel,
            attribution_source,
            source_platform,
            source_detection,
            entry_type,
            entry_route: crate::normalize_path(landing.path()),
            entry_space_id,
        })
    }

    /// Serialize the reviewed attribution fields as PostHog properties.
    pub fn properties(&self) -> Value {
        serde_json::to_value(self).expect("Attribution has an infallible JSON shape")
    }
}

/// Add organic referral attribution to a URL for `space_key`.
///
/// Existing query parameters and the fragment are preserved. Reserved channel
/// and space fields are replaced rather than duplicated.
pub fn space_referral_url(url: &str, space_key: &str) -> Result<String, url::ParseError> {
    let mut parsed = Url::parse(url)?;
    let existing: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(key, _)| key != CHANNEL_PARAMETER && key != SPACE_PARAMETER)
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    parsed.set_query(None);
    {
        let mut query = parsed.query_pairs_mut();
        query.extend_pairs(existing);
        query.append_pair(CHANNEL_PARAMETER, "reshare");
        query.append_pair(SPACE_PARAMETER, &crate::anonymize(space_key));
    }
    Ok(parsed.into())
}

/// Turn a copied `/space/{key}` product URL into an organic referral URL.
/// Other routes are returned unchanged.
pub fn space_route_referral_url(url: &str) -> Result<String, url::ParseError> {
    let parsed = Url::parse(url)?;
    let Some(space_key) = space_key(parsed.path()).map(str::to_owned) else {
        return Ok(parsed.into());
    };
    space_referral_url(parsed.as_str(), &space_key)
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
    properties.insert("schema_version".to_owned(), Value::from(2));
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

fn source_attribution(
    landing: &Url,
    referrer_host: Option<&str>,
) -> (SourcePlatform, SourceDetection) {
    if let Some(value) = landing
        .query_pairs()
        .find(|(key, _)| key == SOURCE_PARAMETER)
        .map(|(_, value)| value.into_owned())
    {
        return (
            canonical_platform(&value).unwrap_or(SourcePlatform::Other),
            SourceDetection::UrlParameter,
        );
    }
    if let Some(value) = landing
        .query_pairs()
        .find(|(key, _)| key == "utm_source")
        .map(|(_, value)| value.into_owned())
    {
        return (
            canonical_platform(&value).unwrap_or(SourcePlatform::Other),
            SourceDetection::Utm,
        );
    }
    match referrer_host {
        Some(host) => (
            platform_from_host(host).unwrap_or(SourcePlatform::Other),
            SourceDetection::Referrer,
        ),
        None => (SourcePlatform::Direct, SourceDetection::Direct),
    }
}

fn canonical_platform(value: &str) -> Option<SourcePlatform> {
    match normalized(value).as_str() {
        "direct" => Some(SourcePlatform::Direct),
        "email" | "newsletter" => Some(SourcePlatform::Email),
        "search" | "google" | "bing" | "duckduckgo" | "brave_search" | "yahoo" => {
            Some(SourcePlatform::Search)
        }
        "x" | "twitter" | "t_co" => Some(SourcePlatform::X),
        "linkedin" => Some(SourcePlatform::Linkedin),
        "instagram" => Some(SourcePlatform::Instagram),
        "facebook" => Some(SourcePlatform::Facebook),
        "reddit" => Some(SourcePlatform::Reddit),
        "discord" => Some(SourcePlatform::Discord),
        "slack" => Some(SourcePlatform::Slack),
        "telegram" => Some(SourcePlatform::Telegram),
        "whatsapp" => Some(SourcePlatform::Whatsapp),
        "bluesky" | "bsky" => Some(SourcePlatform::Bluesky),
        "mastodon" => Some(SourcePlatform::Mastodon),
        "github" => Some(SourcePlatform::Github),
        "product_hunt" | "producthunt" => Some(SourcePlatform::ProductHunt),
        "hacker_news" | "hackernews" | "hn" => Some(SourcePlatform::HackerNews),
        "other" => Some(SourcePlatform::Other),
        _ => None,
    }
}

fn platform_from_host(host: &str) -> Option<SourcePlatform> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let domain = |expected: &str| host == expected || host.ends_with(&format!(".{expected}"));

    if domain("mail.google.com")
        || domain("outlook.live.com")
        || domain("outlook.office.com")
        || domain("mail.yahoo.com")
        || domain("proton.me")
    {
        Some(SourcePlatform::Email)
    } else if domain("x.com") || domain("twitter.com") || domain("t.co") {
        Some(SourcePlatform::X)
    } else if domain("linkedin.com") || domain("lnkd.in") {
        Some(SourcePlatform::Linkedin)
    } else if domain("instagram.com") {
        Some(SourcePlatform::Instagram)
    } else if domain("facebook.com") || domain("fb.com") || domain("fb.me") {
        Some(SourcePlatform::Facebook)
    } else if domain("reddit.com") || domain("redd.it") {
        Some(SourcePlatform::Reddit)
    } else if domain("discord.com") || domain("discordapp.com") {
        Some(SourcePlatform::Discord)
    } else if domain("slack.com") {
        Some(SourcePlatform::Slack)
    } else if domain("telegram.org") || domain("t.me") {
        Some(SourcePlatform::Telegram)
    } else if domain("whatsapp.com") || domain("wa.me") {
        Some(SourcePlatform::Whatsapp)
    } else if domain("bsky.app") {
        Some(SourcePlatform::Bluesky)
    } else if domain("mastodon.social") {
        Some(SourcePlatform::Mastodon)
    } else if domain("github.com") {
        Some(SourcePlatform::Github)
    } else if domain("producthunt.com") {
        Some(SourcePlatform::ProductHunt)
    } else if domain("news.ycombinator.com") {
        Some(SourcePlatform::HackerNews)
    } else if domain("google.com")
        || host.starts_with("www.google.")
        || domain("bing.com")
        || domain("duckduckgo.com")
        || domain("search.brave.com")
        || host.starts_with("search.yahoo.")
    {
        Some(SourcePlatform::Search)
    } else {
        None
    }
}

fn tagged_space(url: &Url) -> Option<String> {
    url.query_pairs()
        .find(|(key, _)| key == SPACE_PARAMETER)
        .map(|(_, value)| value.into_owned())
        .filter(|value| value.len() == 16 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(|value| value.to_ascii_lowercase())
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
        assert_eq!(attribution.source_platform, SourcePlatform::Other);
        assert_eq!(attribution.source_detection, SourceDetection::Utm);
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
        assert_eq!(shared.source_platform, SourcePlatform::Other);
        assert_eq!(shared.source_detection, SourceDetection::Referrer);
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

    #[dialog_common::test]
    fn source_platform_uses_explicit_then_utm_then_referrer_evidence() {
        let explicit = Attribution::from_urls(
            "https://tonk.network/?tonk_source=linkedin&utm_source=reddit",
            Some("https://t.co/thread"),
        )
        .unwrap();
        assert_eq!(explicit.source_platform, SourcePlatform::Linkedin);
        assert_eq!(explicit.source_detection, SourceDetection::UrlParameter);

        let utm = Attribution::from_urls(
            "https://tonk.network/?utm_source=whatsapp",
            Some("https://reddit.com/r/tonk"),
        )
        .unwrap();
        assert_eq!(utm.source_platform, SourcePlatform::Whatsapp);
        assert_eq!(utm.source_detection, SourceDetection::Utm);

        let referrer = Attribution::from_urls(
            "https://tonk.network/",
            Some("https://www.reddit.com/r/tonk/comments/private"),
        )
        .unwrap();
        assert_eq!(referrer.source_platform, SourcePlatform::Reddit);
        assert_eq!(referrer.source_detection, SourceDetection::Referrer);

        let direct = Attribution::from_urls("https://tonk.network/", None).unwrap();
        assert_eq!(direct.source_platform, SourcePlatform::Direct);
        assert_eq!(direct.source_detection, SourceDetection::Direct);
    }

    #[dialog_common::test]
    fn unknown_sources_are_reduced_without_leaking_the_host_or_tag() {
        let tagged = Attribution::from_urls(
            "https://tonk.network/?tonk_source=secret-partner-name",
            Some("https://private.community.example/thread"),
        )
        .unwrap();
        assert_eq!(tagged.source_platform, SourcePlatform::Other);
        assert_eq!(tagged.source_detection, SourceDetection::UrlParameter);
        let payload = visit_properties(&tagged).to_string();
        assert!(!payload.contains("secret-partner-name"));
        assert!(!payload.contains("private.community.example"));
    }

    #[dialog_common::test]
    fn referral_urls_preserve_credentials_and_attribute_join_visits_to_the_space() {
        let raw_space = "did:key:z6MkPrivateSpace";
        let url = space_referral_url(
            "https://tonk.network/join?access=private-proof&revocation=https%3A%2F%2Frelay#private-seed",
            raw_space,
        )
        .unwrap();
        let parsed = Url::parse(&url).unwrap();
        assert_eq!(parsed.fragment(), Some("private-seed"));
        assert_eq!(
            query_value(&parsed, "access").as_deref(),
            Some("private-proof")
        );
        assert_eq!(
            query_value(&parsed, CHANNEL_PARAMETER).as_deref(),
            Some("reshare")
        );
        assert_eq!(
            query_value(&parsed, SPACE_PARAMETER).as_deref(),
            Some(crate::anonymize(raw_space).as_str())
        );
        assert!(!url.contains(raw_space));

        let attribution = Attribution::from_urls(&url, Some("https://t.co/link")).unwrap();
        assert_eq!(
            attribution.entry_space_id.as_deref(),
            Some(crate::anonymize(raw_space).as_str())
        );
        assert_eq!(attribution.source_platform, SourcePlatform::X);
        let payload = visit_properties(&attribution).to_string();
        for sentinel in [raw_space, "private-proof", "private-seed", "t.co"] {
            assert!(!payload.contains(sentinel), "payload leaked {sentinel}");
        }
    }

    #[dialog_common::test]
    fn visible_space_route_wins_and_invalid_space_tags_are_ignored() {
        let route_space = "did:key:z6MkVisibleSpace";
        let routed = Attribution::from_urls(
            &format!("https://tonk.network/space/{route_space}?tonk_space=0000000000000000"),
            None,
        )
        .unwrap();
        assert_eq!(
            routed.entry_space_id.as_deref(),
            Some(crate::anonymize(route_space).as_str())
        );

        for invalid in ["raw-space-name", "abcd", "00000000000000000"] {
            let landing = format!("https://tonk.network/join?tonk_space={invalid}");
            assert_eq!(
                Attribution::from_urls(&landing, None)
                    .unwrap()
                    .entry_space_id,
                None
            );
        }
    }

    #[dialog_common::test]
    fn copied_space_routes_become_organic_referral_links() {
        let raw_space = "did:key:z6MkCopiedSpace";
        let url =
            space_route_referral_url(&format!("https://tonk.network/space/{raw_space}#section"))
                .unwrap();
        let parsed = Url::parse(&url).unwrap();
        assert_eq!(parsed.fragment(), Some("section"));
        assert_eq!(
            query_value(&parsed, CHANNEL_PARAMETER).as_deref(),
            Some("reshare")
        );
        assert_eq!(
            query_value(&parsed, SPACE_PARAMETER).as_deref(),
            Some(crate::anonymize(raw_space).as_str())
        );
        assert_eq!(
            space_route_referral_url("https://tonk.network/settings").unwrap(),
            "https://tonk.network/settings"
        );
    }

    fn query_value(url: &Url, name: &str) -> Option<String> {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    }
}
