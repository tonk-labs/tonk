//! Shared native-only artifact checks performed after measured browser endpoints.

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::path::Path;
use thirtyfour::prelude::*;

use crate::helpers::TestEnvironment;

pub(crate) fn verify_copy(source: &Path, copy: &Path) -> Result<()> {
    let members = |root: &Path| -> Result<Vec<_>> {
        let mut members = std::fs::read_dir(root)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<std::io::Result<Vec<_>>>()?;
        members.sort();
        Ok(members)
    };
    ensure!(
        members(source)? == members(copy)?,
        "served deployment has a different directory member set"
    );
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let other = copy.join(entry.file_name());
        if entry.path().is_dir() {
            verify_copy(&entry.path(), &other)?;
        } else {
            ensure!(
                std::fs::read(entry.path())? == std::fs::read(other)?,
                "served deployment differs from supplied artifact"
            );
        }
    }
    Ok(())
}

/// Verify browser, worker, deployment tree and viewport against the supplied artifact.
/// Call after all measured operations so identity reads cannot warm the sample.
pub(crate) async fn verify_artifact(
    env: &TestEnvironment,
    driver: &WebDriver,
    width: u64,
    height: u64,
) -> Result<Value> {
    driver.enter_default_frame().await?;
    let identity = driver
        .execute_async(
            r#"
        const done = arguments[arguments.length - 1];
        Promise.all(['/version.json', '/asset-manifest.json', '/service_worker.js'].map(
            path => fetch(path, {cache:'no-store'}).then(r => {
                if (!r.ok) throw new Error('identity fetch failed'); return r.text();
            })
        )).then(async files => done({files,
            health: await fetch('/api/health').then(r => r.json()),
            documentBuild: document.querySelector('meta[name="tonk-worker-build"]')?.content,
            viewport: [innerWidth, innerHeight]
        })).catch(() => done({error: 'identity_fetch_failed'}));
    "#,
            vec![],
        )
        .await?;
    let artifact = std::fs::canonicalize(
        std::env::var_os("TONK_UI_RELEASE_ARTIFACT")
            .context("TONK_UI_RELEASE_ARTIFACT required")?,
    )?;
    verify_copy(&artifact, &env.deployment_root.join("generation-a"))?;
    for (index, member) in ["version.json", "asset-manifest.json", "service_worker.js"]
        .iter()
        .enumerate()
    {
        ensure!(
            identity.json()["files"][index].as_str()
                == Some(std::fs::read_to_string(artifact.join(member))?.as_str()),
            "browser artifact identity mismatch for {member}"
        );
    }
    let version: Value = serde_json::from_slice(&std::fs::read(artifact.join("version.json"))?)?;
    let build = version["build"]
        .as_str()
        .context("artifact has no build ID")?;
    ensure!(
        build.len() == 16
            && build
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
        "malformed build ID"
    );
    ensure!(
        identity.json()["documentBuild"] == version["build"],
        "document build mismatch"
    );
    ensure!(
        identity.json()["health"]["build"] == version["build"],
        "active worker build mismatch"
    );
    ensure!(
        identity.json()["viewport"] == json!([width, height]),
        "viewport override mismatch"
    );
    Ok(version)
}
