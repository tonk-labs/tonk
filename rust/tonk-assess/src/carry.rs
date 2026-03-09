//! Carry space provisioning for benchmark runs.
//!
//! Provisions per-persona `.carry/` site directories for benchmarking.
//! Shells out to the `carry` CLI to create spaces and load data.
//!
//! The `carry` binary is resolved via the `CARRY_BIN` environment variable.
//! If unset, falls back to `"carry"` on PATH.

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// Resolve the path to the `carry` binary.
///
/// Checks `CARRY_BIN` first (useful for development when the installed
/// binary is stale). Falls back to bare `"carry"` on PATH.
fn carry_bin() -> String {
    std::env::var("CARRY_BIN").unwrap_or_else(|_| "carry".to_string())
}

/// Run a `carry` command and return its output, with context on failure.
async fn run_carry(args: &[impl AsRef<OsStr>], description: &str) -> Result<std::process::Output> {
    let bin = carry_bin();
    Command::new(&bin)
        .args(args)
        .output()
        .await
        .with_context(|| {
            format!(
                "Failed to run '{bin} {}' — {description}",
                args_display(args)
            )
        })
}

/// Format args for error messages.
fn args_display(args: &[impl AsRef<OsStr>]) -> String {
    args.iter()
        .map(|a| a.as_ref().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Check that the `carry` CLI is available.
///
/// Call this once before running any carry probes. Fails fast with a
/// clear message rather than letting each probe fail individually.
pub async fn ensure_available(verbose: bool) -> Result<()> {
    let bin = carry_bin();
    if verbose {
        eprintln!("[carry] Using carry binary: {bin}");
    }

    // Verify the carry binary is accessible by running --help
    let output = run_carry(&["--help"], "is the carry CLI installed?").await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "carry --help failed (exit {}). Is the carry CLI installed?\n\
             stderr: {stderr}",
            output.status
        );
    }

    if verbose {
        eprintln!("[carry] carry binary verified");
    }

    Ok(())
}

/// Derive a site directory path for a persona within the workspace.
fn persona_site_dir(workspace: &Path, persona: &str) -> PathBuf {
    workspace.join(format!("assess-{persona}"))
}

/// Provision a Carry space for a persona.
///
/// 1. Creates a fresh per-persona site directory.
/// 2. Runs `carry init` to set up the `.carry/` repository.
/// 3. If a model file is provided, asserts it via `carry assert`.
/// 4. Loads the data file via `carry assert`.
///
/// Returns the site directory path on success.
pub async fn provision_space(
    persona: &str,
    model_file: Option<&Path>,
    data_file: &Path,
    workspace: &Path,
    verbose: bool,
) -> Result<PathBuf> {
    let site_dir = persona_site_dir(workspace, persona);

    // Step 1: Remove existing site directory (clean slate)
    if site_dir.exists() {
        if verbose {
            eprintln!(
                "[carry] Removing existing site at '{}'...",
                site_dir.display()
            );
        }
        std::fs::remove_dir_all(&site_dir)
            .with_context(|| format!("Failed to remove {}", site_dir.display()))?;
    }

    // Step 2: Create site directory and init
    std::fs::create_dir_all(&site_dir)
        .with_context(|| format!("Failed to create {}", site_dir.display()))?;

    let site_str = site_dir.to_string_lossy();

    if verbose {
        eprintln!("[carry] Initializing space for persona '{persona}'...");
    }
    let init_output = run_carry(&["--site", &*site_str, "init", persona], "init space").await?;

    if !init_output.status.success() {
        let stderr = String::from_utf8_lossy(&init_output.stderr);
        anyhow::bail!("Failed to init space for persona '{persona}': {stderr}");
    }

    if verbose {
        let stdout = String::from_utf8_lossy(&init_output.stdout);
        eprintln!("[carry] Initialized: {stdout}");
    }

    // Step 3: Import model schema (if provided)
    if let Some(model) = model_file {
        let model_str = model.to_string_lossy();
        if verbose {
            eprintln!("[carry] Asserting model from '{model_str}'...");
        }
        let assert_output = run_carry(
            &["--site", &*site_str, "assert", &*model_str],
            "assert model schema",
        )
        .await?;

        if !assert_output.status.success() {
            let stderr = String::from_utf8_lossy(&assert_output.stderr);
            let stdout = String::from_utf8_lossy(&assert_output.stdout);
            anyhow::bail!(
                "Failed to assert model for persona '{persona}' from '{model_str}':\n\
                 stderr: {stderr}\nstdout: {stdout}"
            );
        }

        if verbose {
            let stdout = String::from_utf8_lossy(&assert_output.stdout);
            eprintln!("[carry] Asserted model: {stdout}");
        }
    }

    // Step 4: Load data from YAML file
    let data_path_str = data_file.to_string_lossy();
    if verbose {
        eprintln!("[carry] Asserting data from '{data_path_str}'...");
    }
    let data_output = run_carry(
        &["--site", &*site_str, "assert", &*data_path_str],
        "assert data",
    )
    .await?;

    if !data_output.status.success() {
        let stderr = String::from_utf8_lossy(&data_output.stderr);
        let stdout = String::from_utf8_lossy(&data_output.stdout);
        anyhow::bail!(
            "Failed to assert data for persona '{persona}' from '{data_path_str}':\n\
             stderr: {stderr}\nstdout: {stdout}"
        );
    }

    if verbose {
        let stdout = String::from_utf8_lossy(&data_output.stdout);
        eprintln!("[carry] Asserted data: {stdout}");
    }

    Ok(site_dir)
}

/// Provision all unique Carry spaces needed by the matched probes.
///
/// Iterates over probes, collects unique `(persona, carry_data, carry_model)` tuples,
/// and provisions a space for each. Returns a tuple of:
/// - The set of personas that were successfully provisioned
/// - A list of `(persona, error)` for personas that failed
///
/// Provisioning is resilient: a failure for one persona does not prevent
/// other personas from being provisioned.
pub async fn provision_all(
    probes: &[crate::types::Probe],
    probe_dir: &Path,
    verbose: bool,
) -> (HashSet<String>, Vec<(String, anyhow::Error)>) {
    let mut provisioned: HashSet<String> = HashSet::new();
    let mut failures: Vec<(String, anyhow::Error)> = Vec::new();
    let mut attempted: HashSet<String> = HashSet::new();

    // Create a workspace directory for all persona sites
    let workspace = std::env::temp_dir().join("carry-assess");
    if let Err(e) = std::fs::create_dir_all(&workspace) {
        failures.push(("_workspace".to_string(), e.into()));
        return (provisioned, failures);
    }

    for probe in probes {
        if let Some(ref carry_data) = probe.carry_data {
            if provisioned.contains(&probe.persona) || attempted.contains(&probe.persona) {
                continue;
            }
            attempted.insert(probe.persona.clone());

            match provision_one_persona(probe, carry_data, probe_dir, &workspace, verbose).await {
                Ok(site_dir) => {
                    println!(
                        "  Space for '{}' ready at {}.",
                        probe.persona,
                        site_dir.display()
                    );
                    provisioned.insert(probe.persona.clone());
                }
                Err(e) => {
                    failures.push((probe.persona.clone(), e));
                }
            }
        }
    }

    (provisioned, failures)
}

/// Provision a single persona's Carry space, resolving paths and calling
/// `provision_space`. Extracted so `provision_all` can catch errors per-persona.
async fn provision_one_persona(
    probe: &crate::types::Probe,
    carry_data: &str,
    probe_dir: &Path,
    workspace: &Path,
    verbose: bool,
) -> Result<PathBuf> {
    let data_path = probe_dir.join(carry_data);
    if !data_path.exists() {
        anyhow::bail!(
            "Carry data file not found for probe '{}': {}\n\
             (resolved from carry-data: '{}')",
            probe.id,
            data_path.display(),
            carry_data,
        );
    }

    let model_path = probe.carry_model.as_ref().map(|m| probe_dir.join(m));
    if let Some(ref mp) = model_path
        && !mp.exists()
    {
        anyhow::bail!(
            "Carry model file not found for probe '{}': {}\n\
             (resolved from carry-model: '{}')",
            probe.id,
            mp.display(),
            probe.carry_model.as_deref().unwrap_or(""),
        );
    }

    println!(
        "Provisioning Carry space for persona '{}'...",
        probe.persona
    );
    provision_space(
        &probe.persona,
        model_path.as_deref(),
        &data_path,
        workspace,
        verbose,
    )
    .await
}

/// Set the active Carry site for a persona.
///
/// Returns the `--site` argument that should be passed to carry commands.
pub fn site_arg_for_persona(persona: &str) -> String {
    let workspace = std::env::temp_dir().join("carry-assess");
    let site_dir = persona_site_dir(&workspace, persona);
    site_dir.to_string_lossy().into_owned()
}
