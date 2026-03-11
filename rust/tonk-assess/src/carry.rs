//! Carry space provisioning for benchmark runs.
//!
//! Shells out to the `carry` CLI to create spaces, import model schemas,
//! and load EAV data before running carry-tagged probes.
//!
//! The `carry` binary is resolved via the `CARRY_BIN` environment variable.
//! If unset, falls back to `"carry"` on PATH.

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::Path;
use tokio::process::Command;

const SPACE_PREFIX: &str = "assess-";

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

/// Check that the `carry` CLI is available and has an active session.
///
/// Call this once before running any carry probes. Fails fast with a
/// clear message rather than letting each probe fail individually.
pub async fn ensure_available(verbose: bool) -> Result<()> {
    let bin = carry_bin();
    if verbose {
        eprintln!("[carry] Using carry binary: {bin}");
    }

    let output = run_carry(&["status", "--json"], "is the carry CLI installed?").await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "carry status failed (exit {}). Is there an active session?\n\
             Run 'carry login' and 'carry space create' first.\n\
             stderr: {stderr}",
            output.status
        );
    }

    if verbose {
        let stdout = String::from_utf8_lossy(&output.stdout);
        eprintln!("[carry] carry status: {stdout}");
    }

    Ok(())
}

/// Provision a Carry space for a persona.
///
/// 1. Deletes any existing `assess-{persona}` space (clean slate).
/// 2. Creates a fresh `assess-{persona}` space.
/// 3. If a model file is provided, imports it via `carry import`.
/// 4. Loads the data file via `carry dev fact batch --file`.
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
    let _ = run_carry(
        &["space", "delete", &space_name, "--force"],
        "delete existing space",
    )
    .await;

    // Step 2: Create a fresh space
    if verbose {
        eprintln!("[carry] Creating space '{space_name}'...");
    }
    let create_output =
        run_carry(&["space", "create", &space_name, "--json"], "create space").await?;

    if !create_output.status.success() {
        let stderr = String::from_utf8_lossy(&create_output.stderr);
        anyhow::bail!(
            "Failed to create space '{space_name}': {stderr}\n\
             Ensure you have an active session (run 'carry login' first)."
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
    let set_output = run_carry(&["space", "set", &space_name], "set active space").await?;

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
            run_carry(&["import", &*model_str, "--json"], "import model schema").await?;

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
    let batch_output = run_carry(
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

    for probe in probes {
        if let Some(ref carry_data) = probe.carry_data {
            if provisioned.contains(&probe.persona) || attempted.contains(&probe.persona) {
                continue;
            }
            attempted.insert(probe.persona.clone());

            match provision_one_persona(probe, carry_data, probe_dir, verbose).await {
                Ok(space_name) => {
                    println!("  Space '{}' ready.", space_name);
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
    verbose: bool,
) -> Result<String> {
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
    provision_space(&probe.persona, model_path.as_deref(), &data_path, verbose).await
}

/// Set the active Carry space for a persona.
///
/// Called before running a probe to ensure the correct space is active.
pub async fn set_active_space(persona: &str, verbose: bool) -> Result<()> {
    let space_name = format!("{SPACE_PREFIX}{persona}");

    if verbose {
        eprintln!("[carry] Switching to space '{space_name}'...");
    }

    let output = run_carry(&["space", "set", &space_name], "set active space").await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to set active space to '{space_name}': {stderr}");
    }

    Ok(())
}
