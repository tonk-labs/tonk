//! Release identity: the `manifest.json` a release publishes next to
//! `checksums.txt`, plus the platform/asset naming shared with
//! `install.sh`.

use serde::{Deserialize, Serialize};

/// A release's self-description, published as `manifest.json`.
///
/// `built_at` is display-only — never parsed — so its format can
/// change without breaking older CLIs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Cargo workspace version of the published binaries.
    pub version: String,
    /// Full git SHA the release was built from.
    pub commit: String,
    /// `stable` or `staging`.
    pub channel: String,
    /// RFC3339 build time, for humans reading the file.
    pub built_at: String,
}

/// Release asset slug for the host, or `None` where nothing is
/// published. Mirrors the `uname` mapping in `install.sh`.
pub fn platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("macos-arm64"),
        ("linux", "x86_64") => Some("linux-x86_64"),
        _ => None,
    }
}

/// Archive name for a platform slug, as published on the release.
pub fn asset_name(platform: &str) -> String {
    format!("tonk-{platform}.tar.gz")
}

/// Pull one asset's SHA256 out of a `checksums.txt` body.
///
/// The file is `sha256sum` output: `<hex>  <name>`, where the name
/// may carry a `*` binary-mode marker.
pub fn parse_checksums(text: &str, asset: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        (name == asset).then(|| hash.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_parses_a_checksum_for_the_named_asset() {
        let text = "aaa  tonk-macos-arm64.tar.gz\nbbb  tonk-linux-x86_64.tar.gz\n";
        assert_eq!(
            parse_checksums(text, "tonk-linux-x86_64.tar.gz"),
            Some("bbb".to_owned())
        );
    }

    #[dialog_common::test]
    fn it_parses_a_checksum_with_a_binary_mode_marker() {
        let text = "aaa *tonk-macos-arm64.tar.gz\n";
        assert_eq!(
            parse_checksums(text, "tonk-macos-arm64.tar.gz"),
            Some("aaa".to_owned())
        );
    }

    #[dialog_common::test]
    fn it_returns_none_when_the_asset_is_absent() {
        let text = "aaa  tonk-macos-arm64.tar.gz\n";
        assert_eq!(parse_checksums(text, "tonk-windows-x64.tar.gz"), None);
    }

    #[dialog_common::test]
    fn it_names_the_asset_for_a_platform() {
        assert_eq!(asset_name("macos-arm64"), "tonk-macos-arm64.tar.gz");
    }

    #[dialog_common::test]
    fn it_round_trips_a_manifest() {
        let json = r#"{"version":"0.4.0","commit":"abc","channel":"stable","built_at":"2026-07-16T00:00:00Z"}"#;
        let manifest: Manifest = serde_json::from_str(json).expect("parse");
        assert_eq!(manifest.version, "0.4.0");
        assert_eq!(manifest.commit, "abc");
    }
}
