//! Returning a signed-in user to where they came from.
//!
//! The account page is a top-document route, so opening it leaves the space
//! the user was on. Whoever sends them there carries `next`, a host-relative
//! path to return to; the account element reads it back through
//! [`requested_next`] once its ceremony completes.
//!
//! This used to also park an operation the service worker had refused for
//! want of an account and replay it after sign-up. Nothing is refused for
//! that reason any more: a device has an account from first boot, and
//! creating a space or joining an invite lands on that account directly.

/// The parameter naming where to return to.
const NEXT_PARAM: &str = "next";

/// Whether `next` may be navigated to.
///
/// Host-relative only. A leading `//` is protocol-relative — the browser reads
/// `//evil.test/x` as another origin — so the parameter would otherwise be an
/// open redirect off an ordinary-looking link.
pub(crate) fn is_safe_next(next: &str) -> bool {
    next.starts_with('/') && !next.starts_with("//")
}

/// The `next` this document was asked to return to, when it is safe.
pub(crate) fn requested_next() -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    let query = search.strip_prefix('?')?;
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(name, _)| name == NEXT_PARAM)
        .map(|(_, value)| value.into_owned())
        .filter(|next| is_safe_next(next))
}

/// Finish a sign-in that just happened: go back to wherever the user came
/// from. Answers `Ok(true)` when it navigated.
///
/// Only for the moment a ceremony completes. A page load that merely FINDS
/// an account must not honour `next`: it means "here is the way back", and
/// following it on load would bounce someone who opened their account
/// settings from a space straight out of the page they asked for.
pub(crate) async fn finish() -> Result<bool, String> {
    match requested_next() {
        Some(next) => {
            tonk_host::navigate_to(&next);
            Ok(true)
        }
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_browser);

    /// `next` is a path this origin will navigate to, so it must not be able
    /// to name another one. A protocol-relative value is the whole trick: the
    /// browser reads `//evil.test/x` as an absolute URL.
    #[dialog_common::test]
    fn it_refuses_a_next_that_leaves_the_origin() {
        for safe in ["/", "/space/abc", "/join?x=1#seed"] {
            assert!(is_safe_next(safe), "{safe}");
        }
        for unsafe_next in [
            "//evil.test/x",
            "https://evil.test/x",
            "javascript:alert(1)",
            "space/abc",
            "",
        ] {
            assert!(!is_safe_next(unsafe_next), "{unsafe_next}");
        }
    }
}
