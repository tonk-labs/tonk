//! Best-effort browser device labels for account registration.

use web_sys::window;

/// Derive the current browser's descriptive account device label.
pub(crate) fn current() -> String {
    let Some(window) = window() else {
        return from_navigator("", "", 0);
    };
    let navigator = window.navigator();
    from_navigator(
        &navigator.user_agent().unwrap_or_default(),
        &navigator.platform().unwrap_or_default(),
        navigator.max_touch_points(),
    )
}

fn from_navigator(user_agent: &str, platform: &str, max_touch_points: i32) -> String {
    let browser = if ["Edg/", "EdgA/", "EdgiOS/"]
        .iter()
        .any(|token| user_agent.contains(token))
    {
        "Edge"
    } else if user_agent.contains("OPR/") {
        "Opera"
    } else if user_agent.contains("SamsungBrowser/") {
        "Samsung Internet"
    } else if user_agent.contains("Chrome/") || user_agent.contains("CriOS/") {
        "Chrome"
    } else if user_agent.contains("Firefox/") || user_agent.contains("FxiOS/") {
        "Firefox"
    } else if user_agent.contains("Safari/") {
        "Safari"
    } else {
        "Browser"
    };

    let is_ipad_desktop = platform.contains("MacIntel") && max_touch_points > 1;
    let os = if is_ipad_desktop
        || ["iPhone", "iPad", "iPod"]
            .iter()
            .any(|token| user_agent.contains(token) || platform.contains(token))
    {
        "iOS"
    } else if user_agent.contains("Android") || platform.contains("Android") {
        "Android"
    } else if user_agent.contains("Windows") || platform.contains("Win") {
        "Windows"
    } else if user_agent.contains("CrOS") || platform.contains("CrOS") {
        "Chrome OS"
    } else if ["Macintosh", "Mac OS", "MacIntel"]
        .iter()
        .any(|token| user_agent.contains(token) || platform.contains(token))
    {
        "macOS"
    } else if user_agent.contains("Linux") || platform.contains("Linux") {
        "Linux"
    } else {
        "Unknown OS"
    };

    format!("{browser} on {os}")
}

#[cfg(test)]
mod tests {
    use super::from_navigator;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_derives_browser_and_os_families_from_navigator_metadata() {
        let cases = [
            (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0 Safari/537.36",
                "MacIntel",
                0,
                "Chrome on macOS",
            ),
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/138.0 Safari/537.36 Edg/138.0",
                "Win32",
                0,
                "Edge on Windows",
            ),
            (
                "Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0",
                "Linux x86_64",
                0,
                "Firefox on Linux",
            ),
            (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 Version/18.5 Safari/605.1.15",
                "MacIntel",
                0,
                "Safari on macOS",
            ),
            (
                "Mozilla/5.0 (Linux; Android 15) AppleWebKit/537.36 Chrome/138.0 Mobile Safari/537.36",
                "Linux armv8l",
                5,
                "Chrome on Android",
            ),
            (
                "Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) AppleWebKit/605.1.15 Version/18.5 Mobile Safari/604.1",
                "iPhone",
                5,
                "Safari on iOS",
            ),
            (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15) AppleWebKit/605.1.15 Version/18.0 Mobile Safari/604.1",
                "MacIntel",
                5,
                "Safari on iOS",
            ),
            ("", "", 0, "Browser on Unknown OS"),
        ];

        for (user_agent, platform, touch_points, expected) in cases {
            assert_eq!(
                from_navigator(user_agent, platform, touch_points),
                expected,
                "user agent: {user_agent}"
            );
        }
    }

    #[dialog_common::test]
    fn it_applies_embedded_browser_token_precedence() {
        let cases = [
            ("Chrome/120 Edg/120", "Edge"),
            ("Chrome/120 EdgA/120", "Edge"),
            ("CriOS/120 EdgiOS/120", "Edge"),
            ("Chrome/120 OPR/106", "Opera"),
            ("Chrome/120 SamsungBrowser/25", "Samsung Internet"),
            ("CriOS/120 Mobile", "Chrome"),
            ("FxiOS/120 Mobile", "Firefox"),
        ];

        for (user_agent, browser) in cases {
            assert_eq!(
                from_navigator(user_agent, "", 0),
                format!("{browser} on Unknown OS"),
                "user agent: {user_agent}"
            );
        }
    }

    #[dialog_common::test]
    fn it_applies_os_token_precedence() {
        let cases = [
            ("iPhone Android Windows CrOS Macintosh Linux", "iOS"),
            ("Android Windows CrOS Macintosh Linux", "Android"),
            ("Windows CrOS Macintosh Linux", "Windows"),
            ("CrOS Macintosh Linux", "Chrome OS"),
            ("Macintosh Linux", "macOS"),
            ("Linux", "Linux"),
        ];

        for (metadata, os) in cases {
            assert_eq!(
                from_navigator(metadata, metadata, 0),
                format!("Browser on {os}"),
                "metadata: {metadata}"
            );
        }
    }
}
