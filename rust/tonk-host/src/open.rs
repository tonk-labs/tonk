//! Opening a link on a guest's behalf — the single policy point.
//!
//! A click inside a sealed guest cannot open anything: the sandbox is
//! `allow-scripts allow-forms`, with no `allow-popups` and no
//! `allow-top-navigation`. The guest relays the raw href and the page
//! decides here.
//!
//! The href is ATTACKER-CONTROLLED. Spot content is data: views and
//! components are facts a collaborator or an agent can assert into a
//! space. This module is where an untrusted string meets the real
//! origin, so two rules are absolute:
//!
//! 1. Parse, never prefix-match. `javascript:` reaching an anchor would
//!    execute on the real origin and defeat the sandbox entirely, and
//!    `JaVaScRiPt:` / leading whitespace / embedded newlines all defeat
//!    string comparison. The URL parser normalises every one of them
//!    into a canonical `protocol`.
//! 2. Never interpolate the URL or host into HTML. Text nodes only.
//!
//! The dialog gates LEAVING THE ORIGIN, not opening a tab: a cmd-clicked
//! in-app link opens silently, an external link is always announced.

use web_sys::Url;

/// Schemes a relayed href may carry. Everything else is rejected.
///
/// `http`/`https` are the point. `mailto`/`tel` are here because they are
/// inert handoffs to an external handler and carry no script. Nothing else
/// has a use case, and every addition is a new way to reach the real origin.
const ALLOWED_SCHEMES: [&str; 4] = ["http:", "https:", "mailto:", "tel:"];

/// What the page decided a relayed href is.
//
// The only caller outside this module's tests is `open_external`, which
// lands with the dialog. Until then the lib target has no user and
// `dead_code` fires on an item the tests exercise heavily.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Destination {
    /// Our own origin — open a tab with no dialog.
    SameOrigin(String),
    /// Off-origin, on the allowlist — confirm first. `host` is what the
    /// dialog names; for `mailto:`/`tel:` it is the address, which is the
    /// only meaningful thing to show.
    External { url: String, host: String },
    /// Not openable.
    Rejected,
}

/// Resolve `href` against `base` and decide what it is.
///
/// `page_origin` must be a canonically serialised origin — i.e.
/// `window.location.origin`, which is what `Url::origin()` is compared
/// against. Anything else fails closed: a mismatch classifies our own
/// origin as external, which shows a needless dialog but opens nothing
/// unannounced.
///
/// Split out from the DOM so it can be tested exhaustively — this is the
/// security boundary, and it is worth more tests per line than anything
/// else in the crate.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn classify(href: &str, base: &str, page_origin: &str) -> Destination {
    let Ok(url) = Url::new_with_base(href, base) else {
        return Destination::Rejected;
    };
    let protocol = url.protocol();
    if !ALLOWED_SCHEMES.contains(&protocol.as_str()) {
        return Destination::Rejected;
    }
    // WHAT WE DISPLAY IS EXACTLY WHAT WE OPEN. `https://tonk.example@evil.com/`
    // has a truthful `host` of `evil.com`, but its `href` reads as ours — and a
    // user reads the URL, so a dialog showing both would be spoofed by the very
    // string it exists to warn about. Strip userinfo before anything is derived
    // from the URL and no `Destination` can carry the disguise; the same edit
    // keeps credentials out of a same-origin navigation.
    //
    // This cannot move a URL between origins: userinfo is not part of an
    // origin, so `origin()` below reads the same either way.
    //
    // BOTH setters are required. Clearing the username alone rewrites
    // `https://u:pw@evil.com/` to `https://:pw@evil.com/` — the password, and
    // the `@` that does the disguising, both survive.
    url.set_username("");
    url.set_password("");
    // `origin` is `"null"` for `mailto:`/`tel:` (opaque path, no host), so
    // they can never collide with a real page origin and are always external.
    if protocol == "mailto:" || protocol == "tel:" {
        // The address IS the name in the dialog, and these two schemes are the
        // only ones that can reach here with an empty one: the parser rejects
        // an `http`/`https` URL with no host, but `mailto:` parses happily to
        // an empty path. It opens nothing, and it would ask the user to
        // approve a blank — so it is not openable.
        let address = url.pathname();
        if address.is_empty() {
            return Destination::Rejected;
        }
        return Destination::External {
            host: address,
            url: url.href(),
        };
    }
    if url.origin() == page_origin {
        return Destination::SameOrigin(url.href());
    }
    // `host` comes from the parser, so an IDN homograph is already punycoded
    // (`аpple.com` shows as `xn--pple-43d.com`) — one reason to display the
    // parsed host rather than anything from the href.
    Destination::External {
        host: url.host(),
        url: url.href(),
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    const BASE: &str = "https://tonk.example/space/abc";
    const ORIGIN: &str = "https://tonk.example";

    fn classified(href: &str) -> Destination {
        classify(href, BASE, ORIGIN)
    }

    /// A path resolves against the page and lands on our own origin, so it
    /// opens with no dialog — there is nothing to warn about.
    #[dialog_common::test]
    async fn it_treats_our_own_origin_as_same_origin() {
        assert_eq!(
            classified("/space/def"),
            Destination::SameOrigin("https://tonk.example/space/def".to_owned()),
            "an in-app path should resolve against the page origin"
        );
        assert_eq!(
            classified("https://tonk.example/space/def"),
            Destination::SameOrigin("https://tonk.example/space/def".to_owned()),
            "an absolute URL on our origin is still same-origin"
        );
    }

    /// A different origin is announced before it opens.
    #[dialog_common::test]
    async fn it_treats_another_origin_as_external() {
        assert_eq!(
            classified("https://example.com/docs/x"),
            Destination::External {
                url: "https://example.com/docs/x".to_owned(),
                host: "example.com".to_owned(),
            },
            "a cross-origin https URL should be announced"
        );
        assert_eq!(
            classified("//example.com/docs/x"),
            Destination::External {
                url: "https://example.com/docs/x".to_owned(),
                host: "example.com".to_owned(),
            },
            "a protocol-relative URL inherits the page scheme and is external"
        );
        assert_eq!(
            classified("http://example.com/"),
            Destination::External {
                url: "http://example.com/".to_owned(),
                host: "example.com".to_owned(),
            },
            "plain http is allowed, and is external even on the same host"
        );
    }

    /// `mailto:`/`tel:` have no origin. They are external by construction, and
    /// the address stands in for the host in the dialog.
    #[dialog_common::test]
    async fn it_shows_the_address_as_the_host_for_mail_and_tel() {
        assert_eq!(
            classified("mailto:someone@example.com"),
            Destination::External {
                url: "mailto:someone@example.com".to_owned(),
                host: "someone@example.com".to_owned(),
            },
            "a mailto address should be what the dialog names"
        );
        assert_eq!(
            classified("tel:+15551234567"),
            Destination::External {
                url: "tel:+15551234567".to_owned(),
                host: "+15551234567".to_owned(),
            },
            "a tel number should be what the dialog names"
        );
    }

    /// THE security test. A relayed href reaches the trusted origin, so a
    /// scheme outside the allowlist must never become an openable URL — a
    /// `javascript:` URL opened here would execute on the real origin and
    /// defeat the sandbox the whole architecture exists to maintain.
    ///
    /// Every evasion below defeats a `starts_with` check and is normalised
    /// away by the URL parser, which is exactly why we parse.
    #[dialog_common::test]
    async fn it_rejects_every_scheme_outside_the_allowlist() {
        for href in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "  javascript:alert(1)",
            "java\nscript:alert(1)",
            "java\tscript:alert(1)",
            "\u{0000}javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "blob:https://tonk.example/abc",
            "file:///etc/passwd",
            "vbscript:msgbox(1)",
            "about:blank",
            "ws://tonk.example/socket",
        ] {
            assert_eq!(
                classified(href),
                Destination::Rejected,
                "`{href}` must never be openable"
            );
        }
    }

    /// An unparseable href is rejected rather than guessed at.
    #[dialog_common::test]
    async fn it_rejects_an_unparseable_href() {
        assert_eq!(
            classified("http://"),
            Destination::Rejected,
            "an href the URL parser rejects should be rejected"
        );
    }

    /// The dialog names the host the browser will actually connect to, not
    /// whatever the href is dressed up to look like. Userinfo before the `@`
    /// and a backslash the parser rewrites to `/` both make an href READ as
    /// ours while resolving elsewhere — the whole point of naming
    /// `url.host()` rather than anything lifted out of the raw string.
    #[dialog_common::test]
    async fn it_names_the_real_host_when_the_href_disguises_it() {
        assert_eq!(
            classified("https://tonk.example@evil.com/"),
            Destination::External {
                url: "https://evil.com/".to_owned(),
                host: "evil.com".to_owned(),
            },
            "userinfo is not the host — the connection goes to evil.com"
        );
        assert_eq!(
            classified("https://evil.com\\@tonk.example/"),
            Destination::External {
                url: "https://evil.com/@tonk.example/".to_owned(),
                host: "evil.com".to_owned(),
            },
            "a backslash normalises to `/`, leaving tonk.example in the path"
        );
    }

    /// What the dialog displays must be exactly what the page opens. Userinfo
    /// makes those two disagree: `https://tonk.example@evil.com/` has a truthful
    /// `host` of `evil.com`, but its `href` READS as ours, and a user reads the
    /// URL. Strip it here so no `Destination` can carry a disguise — or, on the
    /// same-origin path, credentials the page would then navigate with.
    #[dialog_common::test]
    async fn it_strips_userinfo_from_the_url_it_carries() {
        assert_eq!(
            classified("https://tonk.example@evil.com/"),
            Destination::External {
                url: "https://evil.com/".to_owned(),
                host: "evil.com".to_owned(),
            },
            "a username disguising the URL as ours must not survive"
        );
        assert_eq!(
            classified("https://tonk.example:hunter2@evil.com/x?a=1#f"),
            Destination::External {
                url: "https://evil.com/x?a=1#f".to_owned(),
                host: "evil.com".to_owned(),
            },
            "a password must not survive either, and the rest of the URL must"
        );
        assert_eq!(
            classified("https://user:pw@tonk.example/x"),
            Destination::SameOrigin("https://tonk.example/x".to_owned()),
            "credentials must not survive into a same-origin navigation"
        );
        assert_eq!(
            classified("https://@evil.com/"),
            Destination::External {
                url: "https://evil.com/".to_owned(),
                host: "evil.com".to_owned(),
            },
            "an empty userinfo leaves no residue"
        );
    }

    /// Stripping userinfo must not move a URL between origins. Userinfo is not
    /// part of an origin, so it cannot — but the whole fix is worthless if it
    /// can, so pin it: the disguised href stays external, and the same-origin
    /// one stays ours.
    #[dialog_common::test]
    async fn it_does_not_let_stripping_userinfo_change_the_origin() {
        for (href, expected) in [
            (
                "https://tonk.example@evil.com/",
                Destination::External {
                    url: "https://evil.com/".to_owned(),
                    host: "evil.com".to_owned(),
                },
            ),
            (
                "https://evil.com@tonk.example/x",
                Destination::SameOrigin("https://tonk.example/x".to_owned()),
            ),
        ] {
            assert_eq!(
                classified(href),
                expected,
                "`{href}` must classify on its host, not its userinfo"
            );
        }
    }

    /// `host` keeps the port; `hostname` drops it. A URL on our hostname at
    /// another port is a DIFFERENT origin, so naming `hostname` would print
    /// `tonk.example` on a dialog for `https://tonk.example:8443` — the one
    /// case where the two getters disagree, and the reason `classify` must
    /// keep using `host`.
    #[dialog_common::test]
    async fn it_keeps_the_port_in_the_host_it_names() {
        assert_eq!(
            classified("https://tonk.example:8443/x"),
            Destination::External {
                url: "https://tonk.example:8443/x".to_owned(),
                host: "tonk.example:8443".to_owned(),
            },
            "another port is another origin, and the port must be visible"
        );
    }

    /// Host case is not significant, so an uppercase host is still ours; an
    /// IDN homograph is punycoded, so a Cyrillic `а` cannot be shown as Latin.
    #[dialog_common::test]
    async fn it_normalises_host_case_and_punycodes_a_homograph() {
        assert_eq!(
            classified("https://TONK.EXAMPLE/space/def"),
            Destination::SameOrigin("https://tonk.example/space/def".to_owned()),
            "host case is not significant — this is still our origin"
        );
        assert_eq!(
            classified("https://\u{0430}pple.com/"),
            Destination::External {
                url: "https://xn--pple-43d.com/".to_owned(),
                host: "xn--pple-43d.com".to_owned(),
            },
            "a homograph should be named in punycode, not as `apple.com`"
        );
    }

    /// A `mailto:`/`tel:` with no address opens nothing, and its `host` is the
    /// empty pathname — a confirmation dialog naming NOTHING. Asking the user
    /// to approve a blank is worse than not asking, so it is not openable.
    #[dialog_common::test]
    async fn it_rejects_a_mail_or_tel_href_with_no_address() {
        assert_eq!(
            classified("mailto:"),
            Destination::Rejected,
            "a mailto with no address would name nothing in the dialog"
        );
        assert_eq!(
            classified("tel:"),
            Destination::Rejected,
            "a tel with no number would name nothing in the dialog"
        );
    }

    /// An opaque page origin serialises as the string `"null"`. No `http`/
    /// `https` URL ever has that origin, and `mailto:`/`tel:` — the only
    /// allowed schemes that do — return before the comparison. So a page that
    /// somehow reports an opaque origin can never take a foreign URL for its
    /// own.
    #[dialog_common::test]
    async fn it_cannot_mistake_a_url_for_an_opaque_page_origin() {
        assert_eq!(
            classify("https://example.com/x", BASE, "null"),
            Destination::External {
                url: "https://example.com/x".to_owned(),
                host: "example.com".to_owned(),
            },
            "`null` must never match a real URL's origin"
        );
        assert_eq!(
            classify("mailto:someone@example.com", BASE, "null"),
            Destination::External {
                url: "mailto:someone@example.com".to_owned(),
                host: "someone@example.com".to_owned(),
            },
            "a mailto is external regardless of the page origin"
        );
    }
}
