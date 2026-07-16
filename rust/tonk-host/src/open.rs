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

use crate::page_effect;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::{Document, Element, HtmlDialogElement, HtmlElement, Url, window};

/// Schemes a relayed href may carry. Everything else is rejected.
///
/// `http`/`https` are the point. `mailto`/`tel` are here because they are
/// inert handoffs to an external handler and carry no script. Nothing else
/// has a use case, and every addition is a new way to reach the real origin.
const ALLOWED_SCHEMES: [&str; 4] = ["http:", "https:", "mailto:", "tel:"];

/// What the page decided a relayed href is.
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

/// Open `href` on behalf of a guest.
///
/// Forwards until it reaches the page (see `page_effect`), then resolves the
/// href against the page's own URL — which is why a guest can send a bare
/// `/path` without knowing its own origin (it has none; it is opaque).
pub fn open_external(href: &str) {
    if page_effect::forward("open", href) {
        return;
    }
    let Some(win) = window() else {
        return;
    };
    let (Ok(base), Ok(page_origin)) = (win.location().href(), win.location().origin()) else {
        return;
    };
    match classify(href, &base, &page_origin) {
        Destination::SameOrigin(url) => open_same_origin(&url),
        Destination::External { url, label } => confirm_then_open(&url, &label),
        Destination::Rejected => {
            // The top page IS the real console, so warn directly. (The
            // `__tonkRuntime` warn channel exists to lift GUEST errors out of
            // an opaque origin that sanitizes them; nothing to lift here.)
            web_sys::console::warn_1(&JsValue::from_str(&format!(
                "tonk: refused to open `{href}` — scheme is not one of {ALLOWED_SCHEMES:?}"
            )));
        }
    }
}

/// Open our own origin in a new tab, with no dialog — there is nothing to
/// warn about.
///
/// Deliberately WITHOUT `noopener`, for two reasons that point the same way:
/// the destination is our own origin, so an opener reference is harmless and
/// ordinary; and `window.open` with `noopener` returns null unconditionally,
/// which would destroy the only signal we have that the popup was blocked.
///
/// Blocking is a live possibility here: unlike the dialog path there is no
/// confirm press, so this depends on the click's transient user activation
/// surviving the relay from the guest. A same-origin destination degrades to
/// a same-tab route change, which is a reasonable outcome — silently doing
/// nothing, the bug this whole change exists to fix, is not.
fn open_same_origin(url: &str) {
    let Some(win) = window() else {
        return;
    };
    match win.open_with_url_and_target(url, "_blank") {
        Ok(Some(_)) => {}
        _ => crate::navigate_to(url),
    }
}

/// Announce an off-origin destination, and open it if the user agrees.
///
/// The Open press is itself a user activation IN THE TOP DOCUMENT, so this
/// path never gambles on activation surviving the relay. The affordance and
/// the mechanism reinforce each other.
fn confirm_then_open(url: &str, label: &str) {
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let Some(body) = document.body() else {
        return;
    };
    ensure_styles(&document);

    let Some(dialog) = build_dialog(&document, label, url) else {
        return;
    };
    let Some(confirm) = dialog.query_selector(".tonk-open__confirm").ok().flatten() else {
        return;
    };
    let Some(cancel) = dialog.query_selector(".tonk-open__cancel").ok().flatten() else {
        return;
    };

    // One `close` listener owns teardown, so Esc, Cancel and Open all unwind
    // through the same path and the buttons only have to decide intent.
    on_event(dialog.unchecked_ref::<Element>(), "close", {
        let dialog = dialog.clone();
        move || {
            dialog.remove();
        }
    });
    on_event(&cancel, "click", {
        let dialog = dialog.clone();
        move || {
            dialog.close();
        }
    });
    on_event(&confirm, "click", {
        let dialog = dialog.clone();
        let document = document.clone();
        let url = url.to_owned();
        move || {
            open_in_new_tab(&document, &url);
            dialog.close();
        }
    });

    let _ = body.append_child(&dialog);
    let _ = dialog.show_modal();
}

/// Attach a listener that ignores its event. Leaked deliberately: the dialog
/// is removed on `close`, which drops the last reference to the element the
/// listeners are attached to, so nothing outlives the dialog.
fn on_event<F: FnMut() + 'static>(target: &Element, event: &str, mut handler: F) {
    let closure = Closure::wrap(Box::new(move |_: web_sys::Event| handler()) as Box<dyn FnMut(_)>);
    let _ = target.add_event_listener_with_callback(event, closure.as_ref().unchecked_ref());
    closure.forget();
}

/// Build the dialog.
///
/// `label` is the destination's identity as `classify` computed it — the full
/// origin for http(s), the address for `mailto:`/`tel:`. Render it as given:
/// re-deriving it from `url` is how the port, scheme, and userinfo spoofs the
/// classifier already closed get reopened.
///
/// Every attacker-controlled string goes in via `set_text_content`. There is
/// no `set_inner_html` here and there must never be: this renders on the real
/// origin, so interpolating a label or URL into markup would be a scripting
/// hole in the trusted document — the exact thing the scheme allowlist exists
/// to prevent, reintroduced one layer down.
fn build_dialog(document: &Document, label: &str, url: &str) -> Option<HtmlDialogElement> {
    let dialog: HtmlDialogElement = document
        .create_element("dialog")
        .ok()?
        .dyn_into::<HtmlDialogElement>()
        .ok()?;
    let _ = dialog.set_attribute("class", "tonk-open");

    let heading = document.create_element("h2").ok()?;
    heading.set_text_content(Some("Open in a new tab?"));

    let label_line = document.create_element("p").ok()?;
    let _ = label_line.set_attribute("class", "tonk-open__label");
    label_line.set_text_content(Some(label)); // text, never HTML

    let url_line = document.create_element("p").ok()?;
    let _ = url_line.set_attribute("class", "tonk-open__url");
    url_line.set_text_content(Some(url)); // text, never HTML

    let actions = document.create_element("div").ok()?;
    let _ = actions.set_attribute("class", "tonk-open__actions");

    let cancel = document.create_element("button").ok()?;
    let _ = cancel.set_attribute("class", "tonk-open__cancel");
    cancel.set_text_content(Some("Cancel"));

    let confirm = document.create_element("button").ok()?;
    let _ = confirm.set_attribute("class", "tonk-open__confirm");
    confirm.set_text_content(Some("Open"));

    let _ = actions.append_child(&cancel);
    let _ = actions.append_child(&confirm);
    let _ = dialog.append_child(&heading);
    let _ = dialog.append_child(&label_line);
    let _ = dialog.append_child(&url_line);
    let _ = dialog.append_child(&actions);
    Some(dialog)
}

/// Open `url` in a new tab by synthesizing an anchor and clicking it.
///
/// An anchor rather than `window.open` because it handles all four allowed
/// schemes uniformly: `window.open("mailto:…")` can strand a blank tab, while
/// an anchor click hands off to the mail client the way a real link does.
///
/// `noopener noreferrer` because this destination is off-origin: `noopener`
/// denies it a handle on our window (reverse tabnabbing), `noreferrer` keeps
/// the spot's URL out of its logs.
fn open_in_new_tab(document: &Document, url: &str) {
    let Ok(anchor) = document.create_element("a") else {
        return;
    };
    let _ = anchor.set_attribute("href", url);
    let _ = anchor.set_attribute("target", "_blank");
    let _ = anchor.set_attribute("rel", "noopener noreferrer");
    let Some(body) = document.body() else {
        return;
    };
    // Some engines only dispatch a click on a connected element.
    let _ = body.append_child(&anchor);
    if let Some(anchor) = anchor.dyn_ref::<HtmlElement>() {
        anchor.click();
    }
    anchor.remove();
}

/// Inject the dialog's stylesheet once.
///
/// Plain DOM and plain CSS, NOT `<wa-dialog>`: the Web Awesome loader is
/// idle-injected rather than eager (see `tonk-ui/index.html`), because its
/// statically-imported chunks would otherwise starve the boot data plane. A
/// `wa-*` component could still be undefined when an early click lands. Every
/// value is `var(--wa-token, literal)` so it matches the theme when loaded and
/// still looks right before it is — the same technique the boot shell uses,
/// and it keeps index.html's "nothing on the top page uses a wa-* component"
/// true.
fn ensure_styles(document: &Document) {
    const STYLE_ID: &str = "tonk-open-style";
    if document.get_element_by_id(STYLE_ID).is_some() {
        return;
    }
    let Ok(style) = document.create_element("style") else {
        return;
    };
    let _ = style.set_attribute("id", STYLE_ID);
    style.set_text_content(Some(
        r#"
dialog.tonk-open {
  border: 1px solid var(--wa-color-neutral-border-normal, #d4d4d8);
  border-radius: var(--wa-border-radius-l, 8px);
  background: var(--wa-color-surface-raised, #fff);
  color: var(--wa-color-text-normal, #18181b);
  font-family: var(--wa-font-family-body, system-ui, sans-serif);
  padding: 1.25rem;
  max-width: min(28rem, calc(100vw - 2rem));
}
dialog.tonk-open::backdrop { background: rgb(0 0 0 / 0.4); }
.tonk-open h2 {
  margin: 0 0 0.75rem;
  font-size: var(--wa-font-size-l, 1.125rem);
}
.tonk-open__label {
  margin: 0 0 0.25rem;
  font-weight: 600;
}
/* The URL is attacker-chosen: it must wrap rather than widen the dialog,
   and it must not be able to push the buttons off-screen. */
.tonk-open__url {
  margin: 0 0 1.25rem;
  color: var(--wa-color-text-quiet, #71717a);
  font-size: var(--wa-font-size-s, 0.875rem);
  overflow-wrap: anywhere;
  max-height: 4.5rem;
  overflow-y: auto;
}
.tonk-open__actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.5rem;
}
.tonk-open button {
  border-radius: var(--wa-border-radius-m, 6px);
  border: 1px solid var(--wa-color-neutral-border-normal, #d4d4d8);
  background: var(--wa-color-neutral-fill-quiet, #f4f4f5);
  color: inherit;
  font: inherit;
  padding: 0.4rem 0.9rem;
  cursor: pointer;
}
.tonk-open__confirm {
  background: var(--wa-color-brand-fill-loud, #3b4a0a);
  border-color: var(--wa-color-brand-fill-loud, #3b4a0a);
  color: var(--wa-color-brand-on-loud, #f4f7e4);
}
"#,
    ));
    if let Some(head) = document.head() {
        let _ = head.append_child(&style);
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

    /// The dialog renders on the REAL origin, so a hostile label or URL must
    /// land as TEXT and never as markup.
    ///
    /// If this ever fails, the scheme allowlist has been outflanked one layer
    /// down: the URL never had to be openable, because merely *describing* it
    /// would have executed it. `set_text_content` is what holds this line, and
    /// a single `set_inner_html` would break it silently — nothing else in the
    /// change would look different.
    ///
    /// `classify` cannot actually produce these strings today (the parser
    /// encodes them). This asserts the dialog is safe on its OWN terms, so it
    /// stays safe if it ever gains another caller.
    #[dialog_common::test]
    async fn it_renders_a_hostile_label_and_url_as_text_not_markup() {
        let document = web_sys::window()
            .expect("a window in the test harness")
            .document()
            .expect("a document in the test harness");
        let hostile_label = "<img src=x onerror=alert(1)>";
        let hostile_url = "https://example.com/<script>alert(1)</script>";

        let dialog =
            build_dialog(&document, hostile_label, hostile_url).expect("the dialog should build");

        assert!(
            dialog.query_selector("img").ok().flatten().is_none(),
            "a hostile label must not become an element"
        );
        assert!(
            dialog.query_selector("script").ok().flatten().is_none(),
            "a hostile url must not become an element"
        );
        assert_eq!(
            dialog
                .query_selector(".tonk-open__label")
                .ok()
                .flatten()
                .and_then(|el| el.text_content()),
            Some(hostile_label.to_owned()),
            "the label should appear verbatim, as text"
        );
        assert_eq!(
            dialog
                .query_selector(".tonk-open__url")
                .ok()
                .flatten()
                .and_then(|el| el.text_content()),
            Some(hostile_url.to_owned()),
            "the url should appear verbatim, as text"
        );
    }

    /// The dialog says what it does. It opens a new tab and leaves the spot
    /// running, so it must not claim the user is leaving.
    #[dialog_common::test]
    async fn it_names_the_action_without_claiming_the_user_leaves() {
        let document = web_sys::window()
            .expect("a window in the test harness")
            .document()
            .expect("a document in the test harness");

        let dialog = build_dialog(&document, "https://example.com", "https://example.com/")
            .expect("a dialog");

        let text = dialog.text_content().unwrap_or_default();
        assert!(
            text.contains("Open in a new tab?"),
            "the dialog should name the action, got: {text}"
        );
        assert!(
            !text.contains("Leave"),
            "the spot stays open, so the dialog must not say the user is leaving"
        );
    }
}
