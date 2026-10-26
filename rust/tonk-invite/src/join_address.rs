//! Join addresses: another host's link, carried to this one.
//!
//! ```text
//! <origin>/join/<link>
//! ```
//!
//! A link opens on the host that minted it: a ticket link is
//! `https://tonk.network/space/{did}#{seed}`, and a browser that follows
//! it lands on tonk.network. A recipient who keeps their account on
//! another host (tonk.foundation, a local build) joins there by putting
//! the whole link after `/join/` on their own host, as `/seed/<url>`
//! carries a definitions URL:
//!
//! ```text
//! https://tonk.foundation/join/https://tonk.network/space/{did}#{seed}
//! ```
//!
//! The link's own query and fragment become the address's, so its seed
//! is still a fragment and never reaches either server. The address is
//! the rest of the path, percent-decoded (a host may encode the `:` of
//! `https:` or of a `did:key`), followed by the query and fragment as
//! they are. A link encoded whole, `#` and all, still reads back, but its
//! seed has then travelled in the path: `/join` builds the address
//! unencoded, and so should anything else.
//!
//! Nothing here decides what the link is. Unwrapped, it is the link it
//! was, and joins as it would on its own host: a ticket is claimed from
//! the access service at the link's origin, and an `access=` invite
//! follows the endpoint signed into its chain.

use anyhow::{Context, Result};
use url::Url;

/// The path prefix a join address's link follows.
const JOIN_PREFIX: &str = "/join/";

/// The link a join address carries, or `None` when `url` is not one.
///
/// A join address is `/join/` followed by an absolute `http` or `https`
/// URL. A bare `/join`, a `/join?access=...` invite, and a path that is
/// not a URL are not join addresses, and none of them is an error.
pub fn carried(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let rest = parsed.path().strip_prefix(JOIN_PREFIX)?;
    let mut text = urlencoding::decode(rest).ok()?.into_owned();
    if let Some(query) = parsed.query() {
        text.push('?');
        text.push_str(query);
    }
    if let Some(fragment) = parsed.fragment() {
        text.push('#');
        text.push_str(fragment);
    }
    link(&text)
}

/// The join address on `origin` that carries `link`.
///
/// # Errors
///
/// Returns an error if `link` is not an absolute `http` or `https` URL,
/// or if the address does not assemble on `origin`.
pub fn carrying(origin: &Url, link: &str) -> Result<String> {
    let link = self::link(link).context("a join address carries an http or https link")?;
    let root = origin
        .join("/")
        .context("the join address has no origin to sit on")?;
    let address = format!("{}{}", root, &JOIN_PREFIX[1..]) + &link;
    Url::parse(&address).context("the join address did not assemble")?;
    Ok(address)
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

    fn seed() -> String {
        bs58::encode([6u8; 32]).into_string()
    }

    /// The link comes back whole, query and fragment included, so it still
    /// reads as the ticket it was and claims from the host that minted it.
    #[dialog_common::test]
    fn it_carries_another_hosts_ticket_link_whole() {
        let link = format!(
            "https://tonk.network/space/{SPACE}?tonk_channel=reshare&tonk_space=238bda785241754e#{}",
            seed()
        );
        let address = format!("https://tonk.foundation/join/{link}");
        let unwrapped = carried(&address).expect("the address carries a link");
        assert_eq!(unwrapped, link);

        let ticket = Ticket::parse_url(&unwrapped)
            .unwrap()
            .expect("the carried link is still a ticket link");
        assert_eq!(ticket.subject().to_string(), SPACE);
        assert_eq!(ticket.remote().as_str(), "https://tonk.network/ucan/");
    }

    /// The seed stays the address's fragment: nothing a request carries
    /// (path and query) holds it.
    #[dialog_common::test]
    fn it_keeps_the_seed_out_of_the_request() {
        let link = format!("https://tonk.network/space/{SPACE}#{}", seed());
        let address = carrying(&Url::parse("https://tonk.foundation/").unwrap(), &link).unwrap();
        let parsed = Url::parse(&address).unwrap();
        assert_eq!(parsed.fragment(), Some(seed().as_str()));
        assert!(!parsed.path().contains(&seed()), "{address}");
        assert_eq!(parsed.query(), None);
    }

    /// A host that percent-encodes the path (as Cloudflare does a
    /// `did:key`'s colons) leaves the same link.
    #[dialog_common::test]
    fn it_reads_a_percent_encoded_path() {
        let link = format!("https://tonk.network/space/{SPACE}#{}", seed());
        let address = format!(
            "https://tonk.foundation/join/https%3A//tonk.network/space/{}#{}",
            SPACE.replace(':', "%3A"),
            seed()
        );
        assert_eq!(carried(&address), Some(link));
    }

    #[dialog_common::test]
    fn it_round_trips_through_carrying() {
        let link = format!(
            "https://tonk.network/join?access=chain&remote=https%3A%2F%2Ftonk.network#{}",
            seed()
        );
        let origin = Url::parse("https://tonk.foundation/settings?tab=links").unwrap();
        let address = carrying(&origin, &link).unwrap();
        assert!(
            address.starts_with("https://tonk.foundation/join/https://tonk.network/join?"),
            "{address}"
        );
        assert_eq!(carried(&address), Some(link));
    }

    /// Everything else that reaches `/join` keeps meaning what it meant:
    /// an `access=` invite is an invite, a bare visit is a visit, and a
    /// path that is not a link is not one.
    #[dialog_common::test]
    fn it_leaves_everything_else_alone() {
        let seed = seed();
        for url in [
            "https://tonk.foundation/join".to_owned(),
            "https://tonk.foundation/join/".to_owned(),
            format!("https://tonk.foundation/join?access=chain#{seed}"),
            format!("https://tonk.foundation/join#https://tonk.network/space/{SPACE}#{seed}"),
            "https://tonk.foundation/join/javascript:alert(1)".to_owned(),
            "https://tonk.foundation/join/not-a-link".to_owned(),
            format!("https://tonk.foundation/joined/https://tonk.network/space/{SPACE}"),
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
