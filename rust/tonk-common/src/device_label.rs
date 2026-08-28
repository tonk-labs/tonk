//! Device labels — "Chrome on macOS".
//!
//! The parsing is pure so both the page and the service worker can use
//! it: the page reads `window.navigator`, the worker reads
//! `WorkerNavigator`, and neither has the other's globals. Shared
//! because a device labelled one way at link time and another way in a
//! device list would look like two devices.

/// Build a "{browser} on {os}" label from navigator metadata.
///
/// Every field is best effort: an empty user agent and platform yield
/// "Browser on Unknown OS" rather than failing, because a device
/// without a good label is still a device worth listing.
pub fn from_navigator(user_agent: &str, platform: &str, max_touch_points: i32) -> String {
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
