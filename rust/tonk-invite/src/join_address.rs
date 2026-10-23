//! Join addresses: another host's link, carried to this one.
//!
//! ```text
//! <origin>/join#<link>
//! ```
//!
//! A link opens on the host that minted it: a ticket link is
//! `https://tonk.network/space/{did}#{seed}`, and a browser that follows
//! it lands on tonk.network. A recipient who keeps their account on
//! another host (tonk.foundation, a local build) joins there by putting
//! the whole link after `/join#` on their own host:
//!
//! ```text
//! https://tonk.foundation/join#https://tonk.network/space/{did}#{seed}
//! ```
//!
//! The link rides in the fragment, so neither host's server sees it: not
//! the space it names, not the seed beside it. A fragment may itself
//! contain `#`, so the link's own fragment is kept as it is. The link may
//! also be percent-encoded, as `encodeURIComponent` leaves it.
//!
//! Nothing here decides what the link is. Unwrapped, it is the link it
//! was, and joins as it would on its own host: a ticket is claimed from
//! the access service at the link's origin, and an `access=` invite
//! follows the endpoint signed into its chain.

use anyhow::{Context, Result};
use url::Url;

/// The path a join address sits at.
const JOIN_PATH: &str = "/join";

/// The link a join address carries, or `None` when `url` is not one.
///
/// A join address is `/join` with a fragment that is an absolute `http`
/// or `https` URL, as written or percent-encoded. A bare `/join`, a
/// `/join?access=...` invite (whose seed is its fragment), and a fragment
/// that is not a URL are not join addresses, and none of them is an error.
pub fn carried(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    if parsed.path() != JOIN_PATH {
        return None;
    }
    let fragment = parsed.fragment()?;
    link(fragment).or_else(|| link(&urlencoding::decode(fragment).ok()?))
}

/// The join address on `origin` that carries `link`.
///
/// # Errors
///
/// Returns an error if `origin` has no `/join` route to resolve, or if
/// `link` is not an absolute `http` or `https` URL.
pub fn carrying(origin: &Url, link: &str) -> Result<String> {
    let link = self::link(link).context("a join address carries an http or https link")?;
    let mut address = origin
        .join(JOIN_PATH)
        .context("the join address did not assemble")?;
    address.set_query(None);
    Ok(format!("{address}#{link}"))
}

/// `text` as an absolute `http` or `https` link, serialized the way it
/// parsed.
fn link(text: &str) -> Option<String> {
    let parsed = Url::parse(text).ok()?;
    matches!(parsed.scheme(), "http" | "https").then(|| parsed.into())
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    use super::{carried, carrying};
    use crate::Ticket;
    use url::Url;

    const SPACE: &str = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

    fn ticket_link() -> String {
        Ticket::new(
            SPACE.parse().unwrap(),
            [6u8; 32],
            &Url::parse("https://tonk.network/ucan/").unwrap(),
        )
        .and_then(|ticket| ticket.to_url())
        .unwrap()
    }

    /// The link comes back whole, its own fragment included, so it still
    /// reads as the ticket it was and claims from the host that minted it.
    #[dialog_common::test]
    fn it_carries_another_hosts_ticket_link_whole() {
        let seed = bs58::encode([6u8; 32]).into_string();
        let link = format!(
            "https://tonk.network/space/{SPACE}?tonk_channel=reshare&tonk_space=238bda785241754e#{seed}"
        );

        let address = format!("https://tonk.foundation/join#{link}");
        let unwrapped = carried(&address).expect("the address carries a link");
        assert_eq!(unwrapped, link);

        let ticket = Ticket::parse_url(&unwrapped)
            .unwrap()
            .expect("the carried link is still a ticket link");
        assert_eq!(ticket.subject().to_string(), SPACE);
        assert_eq!(ticket.remote().as_str(), "https://tonk.network/ucan/");
    }

    /// A browser hands the page its location as `URL.href`, which keeps a
    /// second `#` in the fragment; the `url` crate reads it the same way.
    #[dialog_common::test]
    fn it_keeps_the_links_own_fragment() {
        let link = ticket_link();
        let address = format!("https://tonk.foundation/join#{link}");
        let parsed = Url::parse(&address).unwrap();
        assert_eq!(parsed.path(), "/join");
        assert_eq!(parsed.fragment(), Some(link.as_str()));
        assert_eq!(carried(parsed.as_str()), Some(link));
    }

    #[dialog_common::test]
    fn it_reads_a_percent_encoded_link() {
        let link = ticket_link();
        let address = format!(
            "https://tonk.foundation/join#{}",
            urlencoding::encode(&link)
        );
        assert_eq!(carried(&address), Some(link));
    }

    #[dialog_common::test]
    fn it_round_trips_through_carrying() {
        let link = ticket_link();
        let origin = Url::parse("https://tonk.foundation/settings?tab=links").unwrap();
        let address = carrying(&origin, &link).unwrap();
        assert!(
            address.starts_with("https://tonk.foundation/join#https://tonk.network/space/"),
            "{address}"
        );
        assert_eq!(carried(&address), Some(link));
    }

    /// Everything else that reaches `/join` keeps meaning what it meant:
    /// an `access=` invite's fragment is its seed, a bare visit is a
    /// visit, and a fragment that is not a link is not one.
    #[dialog_common::test]
    fn it_leaves_everything_else_alone() {
        let seed = bs58::encode([6u8; 32]).into_string();
        for url in [
            "https://tonk.foundation/join".to_owned(),
            format!("https://tonk.foundation/join?access=chain#{seed}"),
            format!("https://tonk.foundation/join#{seed}"),
            "https://tonk.foundation/join#javascript:alert(1)".to_owned(),
            "https://tonk.foundation/join#mailto:someone@example.com".to_owned(),
            format!("https://tonk.foundation/space/{SPACE}#{}", ticket_link()),
            format!("https://tonk.foundation/joined#{}", ticket_link()),
            "not a url".to_owned(),
        ] {
            assert_eq!(carried(&url), None, "{url}");
        }
        assert!(
            carrying(
                &Url::parse("https://tonk.foundation/").unwrap(),
                "javascript:alert(1)"
            )
            .is_err()
        );
    }
}
