//! HTTP against a release: manifest, checksums, archive.
//!
//! `TONK_UPDATE_ENDPOINT` repoints the base URL so tests serve a fake
//! release from a local listener.

use anyhow::{Context as _, bail};

use crate::update::{Channel, endpoint, manifest::Manifest};

/// Fetch and parse the channel's `manifest.json`.
pub async fn manifest(channel: Channel) -> anyhow::Result<Manifest> {
    let url = format!("{}/manifest.json", channel.base_url(&endpoint()));
    let response = reqwest::get(&url)
        .await
        .with_context(|| format!("could not reach {url}"))?;
    if !response.status().is_success() {
        bail!(
            "no manifest.json on the {} channel ({} returned {})",
            channel.as_str(),
            url,
            response.status()
        );
    }
    let text = response
        .text()
        .await
        .with_context(|| format!("could not read {url}"))?;
    serde_json::from_str(&text).with_context(|| format!("could not parse {url}"))
}

/// Fetch the channel's raw `checksums.txt`.
pub async fn checksums(channel: Channel) -> anyhow::Result<String> {
    let url = format!("{}/checksums.txt", channel.base_url(&endpoint()));
    let response = reqwest::get(&url)
        .await
        .with_context(|| format!("could not reach {url}"))?;
    if !response.status().is_success() {
        bail!("could not download {url} ({})", response.status());
    }
    response
        .text()
        .await
        .with_context(|| format!("could not read {url}"))
}

/// Download one release archive.
pub async fn archive(channel: Channel, asset: &str) -> anyhow::Result<Vec<u8>> {
    let url = format!("{}/{asset}", channel.base_url(&endpoint()));
    let response = reqwest::get(&url)
        .await
        .with_context(|| format!("could not reach {url}"))?;
    if !response.status().is_success() {
        bail!("could not download {url} ({})", response.status());
    }
    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("could not read {url}"))?;
    Ok(bytes.to_vec())
}
