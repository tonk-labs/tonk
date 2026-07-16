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
//! 2. Never interpolate the URL or the name it is shown under into HTML.
//!    Text nodes only.
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
    /// Off-origin, on the allowlist — confirm first.
    ///
    /// `label` is what the dialog names as the destination's identity: the
    /// full origin (`scheme://host:port`) for `http`/`https`, and the address
    /// for `mailto:`/`tel:`, which have no origin to name.
    External { url: String, label: String },
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
/// `base` must be the page's own absolute URL (`window.location.href`) and
/// agree with `page_origin`. Both ways of getting it wrong also fail closed:
/// an invalid `base` makes every href unparseable and so `Rejected`, and a
/// `base` disagreeing with `page_origin` makes our own links spuriously
/// `External` — noisy, never silent.
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
    // has a truthful origin of `https://evil.com`, but its `href` reads as
    // ours — and a user reads the URL, so a dialog showing both would be
    // spoofed by the very string it exists to warn about. Strip userinfo before
    // anything is derived from the URL and no `Destination` can carry the
    // disguise; the same edit keeps credentials out of a same-origin navigation.
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
    // Their `label` is the address, which means the address must actually BE
    // one — the parser is happy to accept plenty that is not.
    if protocol == "mailto:" || protocol == "tel:" {
        // An authority is the sharp case. `mailto://tonk.example/x` parses
        // with a host and a pathname of `/x`, so it would NAME `/x` while
        // OPENING `mailto://tonk.example/x` — display and destination
        // disagreeing, which is the one thing this must never do. No real
        // mail or tel address has an authority, so its presence is enough.
        //
        // Userinfo is not theoretical here either: the setters above are inert
        // only while the host is null, so `mailto://user:pw@evil.com/x` really
        // does parse with credentials and really is stripped.
        if !url.host().is_empty() {
            return Destination::Rejected;
        }
        let address = url.pathname();
        // A path-shaped address is the same disagreement without the host:
        // `mailto:/x` names `/x`, which is not an address either.
        if address.starts_with('/') {
            return Destination::Rejected;
        }
        // A blank name is worse than no dialog: it asks the user to approve
        // NOTHING. `mailto:` is empty, `mailto:%20` is a space, `tel:.` is
        // bare punctuation — none of them name something a person can judge,
        // and every real address carries a letter or a digit. Decoding is only
        // how we make that judgement; the label stays the raw pathname, so
        // what we display is still exactly what we open. A malformed escape
        // fails to decode and so fails closed.
        let decoded = js_sys::decode_uri_component(&address)
            .map(String::from)
            .unwrap_or_default();
        if !decoded.chars().any(char::is_alphanumeric) {
            return Destination::Rejected;
        }
        return Destination::External {
            label: address,
            url: url.href(),
        };
    }
    if url.origin() == page_origin {
        return Destination::SameOrigin(url.href());
    }
    // Name the ORIGIN, not the host. A host cannot express an origin: it drops
    // the scheme, so `http://tonk.example/` — which is NOT our origin — would
    // be named `tonk.example`, printing our own identity on a dialog warning
    // about leaving us. A plain-http downgrade must not read as home. The
    // origin is also exactly what the comparison above uses, so the dialog
    // names the same thing the decision was made on.
    //
    // It comes from the parser, so an IDN homograph is already punycoded
    // (`аpple.com` names `https://xn--pple-43d.com`) — one reason to display
    // the parsed origin rather than anything lifted from the href.
    Destination::External {
        label: url.origin(),
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
                label: "https://example.com".to_owned(),
            },
            "a cross-origin https URL should be announced"
        );
        assert_eq!(
            classified("//example.com/docs/x"),
            Destination::External {
                url: "https://example.com/docs/x".to_owned(),
                label: "https://example.com".to_owned(),
            },
            "a protocol-relative URL inherits the page scheme and is external"
        );
        assert_eq!(
            classified("http://example.com/"),
            Destination::External {
                url: "http://example.com/".to_owned(),
                label: "http://example.com".to_owned(),
            },
            "plain http is allowed, and is external even on the same host"
        );
    }

    /// `mailto:`/`tel:` have no origin. They are external by construction, and
    /// the address stands in for it as the name in the dialog.
    #[dialog_common::test]
    async fn it_names_the_address_for_mail_and_tel() {
        assert_eq!(
            classified("mailto:someone@example.com"),
            Destination::External {
                url: "mailto:someone@example.com".to_owned(),
                label: "someone@example.com".to_owned(),
            },
            "a mailto address should be what the dialog names"
        );
        assert_eq!(
            classified("tel:+15551234567"),
            Destination::External {
                url: "tel:+15551234567".to_owned(),
                label: "+15551234567".to_owned(),
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

    /// The dialog names the origin the browser will actually connect to, not
    /// whatever the href is dressed up to look like. Userinfo before the `@`
    /// and a backslash the parser rewrites to `/` both make an href READ as
    /// ours while resolving elsewhere — the whole point of naming
    /// `url.origin()` rather than anything lifted out of the raw string.
    #[dialog_common::test]
    async fn it_names_the_real_origin_when_the_href_disguises_it() {
        assert_eq!(
            classified("https://tonk.example@evil.com/"),
            Destination::External {
                url: "https://evil.com/".to_owned(),
                label: "https://evil.com".to_owned(),
            },
            "userinfo is not the host — the connection goes to evil.com"
        );
        assert_eq!(
            classified("https://evil.com\\@tonk.example/"),
            Destination::External {
                url: "https://evil.com/@tonk.example/".to_owned(),
                label: "https://evil.com".to_owned(),
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
                label: "https://evil.com".to_owned(),
            },
            "a username disguising the URL as ours must not survive"
        );
        assert_eq!(
            classified("https://tonk.example:hunter2@evil.com/x?a=1#f"),
            Destination::External {
                url: "https://evil.com/x?a=1#f".to_owned(),
                label: "https://evil.com".to_owned(),
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
                label: "https://evil.com".to_owned(),
            },
            "an empty userinfo leaves no residue"
        );
    }

    /// A userinfo that reads like ANOTHER origin must not push our own URL off
    /// it. `https://evil.com@tonk.example/x` connects to `tonk.example`;
    /// `evil.com` is a username. This is the mirror of the disguise the tests
    /// above pin — there the userinfo reads as ours and the host is hostile,
    /// here it reads as hostile and the host is ours — and both come out right
    /// for the same reason: classification reads the host and nothing else.
    #[dialog_common::test]
    async fn it_classifies_on_the_host_not_the_userinfo() {
        assert_eq!(
            classified("https://evil.com@tonk.example/x"),
            Destination::SameOrigin("https://tonk.example/x".to_owned()),
            "`evil.com` here is a username — the connection is to ours"
        );
    }

    /// An origin carries the port. A URL on our hostname at another port is a
    /// DIFFERENT origin, so a name that dropped the port would print
    /// `tonk.example` on a dialog for `https://tonk.example:8443`. This is the
    /// sibling of the scheme case: every part of the origin has to survive
    /// into the name, or the name can be confused with ours.
    #[dialog_common::test]
    async fn it_keeps_the_port_in_the_origin_it_names() {
        assert_eq!(
            classified("https://tonk.example:8443/x"),
            Destination::External {
                url: "https://tonk.example:8443/x".to_owned(),
                label: "https://tonk.example:8443".to_owned(),
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
                label: "https://xn--pple-43d.com".to_owned(),
            },
            "a homograph should be named in punycode, not as `apple.com`"
        );
    }

    /// A `mailto:`/`tel:` with no address opens nothing, and its `label` is the
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

    /// A scheme downgrade keeps our hostname. `http://tonk.example/` is NOT our
    /// origin, so naming `tonk.example` would print OUR OWN identity on the
    /// dialog for a destination that is not us — a plain-http downgrade reading
    /// as home is exactly the confusion the dialog exists to prevent. The name
    /// must carry the scheme, so it can never be mistaken for our origin.
    #[dialog_common::test]
    async fn it_names_an_origin_a_scheme_downgrade_cannot_disguise() {
        assert_eq!(
            classified("http://tonk.example/"),
            Destination::External {
                url: "http://tonk.example/".to_owned(),
                label: "http://tonk.example".to_owned(),
            },
            "a scheme downgrade on our own hostname must not be named as us"
        );
    }

    /// `mailto:`/`tel:` name their address, so an href whose address is not an
    /// address must not reach the dialog. `mailto://tonk.example/x` is the sharp
    /// one: it would name `/x` while opening `mailto://tonk.example/x` — display
    /// and destination disagreeing, which is the one thing this must never do.
    #[dialog_common::test]
    async fn it_rejects_a_mail_or_tel_href_that_names_a_non_address() {
        for href in [
            "mailto:/x",
            "mailto://tonk.example/x",
            "mailto://user:pw@evil.com/x",
            "mailto:%20",
            "tel:.",
            "tel://evil.com/x",
        ] {
            assert_eq!(
                classified(href),
                Destination::Rejected,
                "`{href}` does not name an address and must not be openable"
            );
        }
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
                label: "https://example.com".to_owned(),
            },
            "`null` must never match a real URL's origin"
        );
        assert_eq!(
            classify("mailto:someone@example.com", BASE, "null"),
            Destination::External {
                url: "mailto:someone@example.com".to_owned(),
                label: "someone@example.com".to_owned(),
            },
            "a mailto is external regardless of the page origin"
        );
    }
}
