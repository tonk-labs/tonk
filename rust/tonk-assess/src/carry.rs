//! Carry space provisioning for benchmark runs.
//!
//! Shells out to the `tonk` CLI to create spaces, import model schemas,
//! and load EAV data before running carry-tagged probes.
//!
//! The `tonk` binary is resolved via the `TONK_BIN` environment variable.
//! If unset, falls back to `"tonk"` on PATH.

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::Path;
use tokio::process::Command;

const SPACE_PREFIX: &str = "assess-";

/// Resolve the path to the `tonk` binary.
///
/// Checks `TONK_BIN` first (useful for development when the installed
/// binary is stale). Falls back to bare `"tonk"` on PATH.
fn tonk_bin() -> String {
    std::env::var("TONK_BIN").unwrap_or_else(|_| "tonk".to_string())
}

/// Run a `tonk` command and return its output, with context on failure.
async fn run_tonk(args: &[impl AsRef<OsStr>], description: &str) -> Result<std::process::Output> {
    let bin = tonk_bin();
    Command::new(&bin)
        .args(args)
        .output()
        .await
        .with_context(|| format!("Failed to run '{bin} {}' — {description}", args_display(args)))
}

/// Format args for error messages.
fn args_display(args: &[impl AsRef<OsStr>]) -> String {
    args.iter()
        .map(|a| a.as_ref().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Check that the `tonk` CLI is available and has an active session.
///
/// Call this once before running any carry probes. Fails fast with a
/// clear message rather than letting each probe fail individually.
pub async fn ensure_available(verbose: bool) -> Result<()> {
    let bin = tonk_bin();
    if verbose {
        eprintln!("[carry] Using tonk binary: {bin}");
    }

    let output = run_tonk(&["status", "--json"], "is the tonk CLI installed?").await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "tonk status failed (exit {}). Is there an active session?\n\
             Run 'tonk login' and 'tonk space create' first.\n\
             stderr: {stderr}",
            output.status
        );
    }

    if verbose {
        let stdout = String::from_utf8_lossy(&output.stdout);
        eprintln!("[carry] tonk status: {stdout}");
    }

    Ok(())
}

/// Provision a Carry space for a persona.
///
/// 1. Deletes any existing `assess-{persona}` space (clean slate).
/// 2. Creates a fresh `assess-{persona}` space.
/// 3. If a model file is provided, imports it via `tonk import`.
/// 4. Loads the data file via `tonk dev fact batch --file`.
///
/// Returns the space name on success.
pub async fn provision_space(
    persona: &str,
    model_file: Option<&Path>,
    data_file: &Path,
    verbose: bool,
) -> Result<String> {
    let space_name = format!("{SPACE_PREFIX}{persona}");

    // Step 1: Delete existing space (ignore errors — it may not exist)
    if verbose {
        eprintln!("[carry] Deleting space '{space_name}' if it exists...");
    }
    let _ = run_tonk(
        &["space", "delete", &space_name, "--force"],
        "delete existing space",
    )
    .await;

    // Step 2: Create a fresh space
    if verbose {
        eprintln!("[carry] Creating space '{space_name}'...");
    }
    let create_output = run_tonk(
        &["space", "create", &space_name, "--json"],
        "create space",
    )
    .await?;

    if !create_output.status.success() {
        let stderr = String::from_utf8_lossy(&create_output.stderr);
        anyhow::bail!(
            "Failed to create space '{space_name}': {stderr}\n\
             Ensure you have an active session (run 'tonk login' first)."
        );
    }

    if verbose {
        let stdout = String::from_utf8_lossy(&create_output.stdout);
        eprintln!("[carry] Created space: {stdout}");
    }

    // Step 3: Set the space as active
    if verbose {
        eprintln!("[carry] Setting active space to '{space_name}'...");
    }
    let set_output = run_tonk(&["space", "set", &space_name], "set active space").await?;

    if !set_output.status.success() {
        let stderr = String::from_utf8_lossy(&set_output.stderr);
        anyhow::bail!("Failed to set active space to '{space_name}': {stderr}");
    }

    // Step 4: Import model schema (if provided)
    if let Some(model) = model_file {
        let model_str = model.to_string_lossy();
        if verbose {
            eprintln!("[carry] Importing model from '{model_str}'...");
        }
        let import_output =
            run_tonk(&["import", &*model_str, "--json"], "import model schema").await?;

        if !import_output.status.success() {
            let stderr = String::from_utf8_lossy(&import_output.stderr);
            let stdout = String::from_utf8_lossy(&import_output.stdout);
            anyhow::bail!(
                "Failed to import model into space '{space_name}' from '{model_str}':\n\
                 stderr: {stderr}\nstdout: {stdout}"
            );
        }

        if verbose {
            let stdout = String::from_utf8_lossy(&import_output.stdout);
            eprintln!("[carry] Imported model: {stdout}");
        }
    }

    // Step 5: Load data from YAML file
    let data_path_str = data_file.to_string_lossy();
    if verbose {
        eprintln!("[carry] Loading data from '{data_path_str}'...");
    }
    let batch_output = run_tonk(
        &["dev", "fact", "batch", "--file", &*data_path_str, "--json"],
        "load data",
    )
    .await?;

    if !batch_output.status.success() {
        let stderr = String::from_utf8_lossy(&batch_output.stderr);
        let stdout = String::from_utf8_lossy(&batch_output.stdout);
        anyhow::bail!(
            "Failed to load data into space '{space_name}' from '{data_path_str}':\n\
             stderr: {stderr}\nstdout: {stdout}"
        );
    }

    if verbose {
        let stdout = String::from_utf8_lossy(&batch_output.stdout);
        eprintln!("[carry] Loaded data: {stdout}");
    }

    Ok(space_name)
}

/// Provision all unique Carry spaces needed by the matched probes.
///
/// Iterates over probes, collects unique `(persona, carry_data, carry_model)` tuples,
/// and provisions a space for each. Returns the set of personas that
/// were successfully provisioned.
pub async fn provision_all(
    probes: &[crate::types::Probe],
    probe_dir: &Path,
    verbose: bool,
) -> Result<HashSet<String>> {
    let mut provisioned: HashSet<String> = HashSet::new();

    for probe in probes {
        if let Some(ref carry_data) = probe.carry_data {
            if provisioned.contains(&probe.persona) {
                continue;
            }

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
            if let Some(ref mp) = model_path {
                if !mp.exists() {
                    anyhow::bail!(
                        "Carry model file not found for probe '{}': {}\n\
                         (resolved from carry-model: '{}')",
                        probe.id,
                        mp.display(),
                        probe.carry_model.as_deref().unwrap_or(""),
                    );
                }
            }

            println!(
                "Provisioning Carry space for persona '{}'...",
                probe.persona
            );
            let space_name = provision_space(
                &probe.persona,
                model_path.as_deref(),
                &data_path,
                verbose,
            )
            .await?;
            println!("  Space '{}' ready.", space_name);

            provisioned.insert(probe.persona.clone());
        }
    }

    Ok(provisioned)
}

/// Set the active Carry space for a persona.
///
/// Called before running a probe to ensure the correct space is active.
pub async fn set_active_space(persona: &str, verbose: bool) -> Result<()> {
    let space_name = format!("{SPACE_PREFIX}{persona}");

    if verbose {
        eprintln!("[carry] Switching to space '{space_name}'...");
    }

    let output = run_tonk(&["space", "set", &space_name], "set active space").await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to set active space to '{space_name}': {stderr}");
    }

    Ok(())
}
