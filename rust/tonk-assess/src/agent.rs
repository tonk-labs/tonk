use anyhow::{Context, Result};
use std::path::Path;
use std::time::Instant;
use tokio::process::Command;

use crate::types::{Probe, RunMetrics};

const DEFAULT_TOOLS: &str = "Read,Glob,Grep";
const DEFAULT_MAX_TURNS: u32 = 10;

pub async fn run_probe(
    probe: &Probe,
    model: &str,
    probe_dir: &Path,
    corpus_dir: &Path,
    verbose: bool,
) -> Result<RunMetrics> {
    let start = Instant::now();

    let max_turns = probe.max_turns.unwrap_or(DEFAULT_MAX_TURNS);

    let mut cmd = Command::new("claude");
    cmd.arg("-p")
        .arg("--output-format")
        .arg("json")
        .arg("--no-session-persistence")
        .arg("--max-turns")
        .arg(max_turns.to_string())
        .arg("--model")
        .arg(model);

    // Clear CLAUDECODE env to avoid inheriting parent session
    cmd.env("CLAUDECODE", "");

    let tools = if probe.allowed_tools.is_empty() {
        DEFAULT_TOOLS.to_string()
    } else {
        probe.allowed_tools.join(",")
    };
    cmd.arg("--allowedTools").arg(&tools);
    cmd.arg("--add-dir").arg(corpus_dir);

    if let Some(ref prompt_path) = probe.system_prompt {
        let resolved = probe_dir.join(prompt_path);
        let content = std::fs::read_to_string(&resolved).with_context(|| {
            format!("failed to read system-prompt file: {}", resolved.display())
        })?;
        cmd.arg("--append-system-prompt").arg(content);
    }

    if let Some(ref mcp) = probe.mcp_config {
        cmd.arg("--mcp-config").arg(mcp);
    }

    cmd.arg("--").arg(&probe.prompt);

    if verbose {
        let args: Vec<_> = std::iter::once("claude".to_string())
            .chain(
                cmd.as_std()
                    .get_args()
                    .map(|a| a.to_string_lossy().to_string()),
            )
            .collect();
        eprintln!("[agent] running: {}", args.join(" "));
    }

    let output = cmd
        .output()
        .await
        .context("failed to spawn claude CLI — is it installed and on PATH?")?;

    let elapsed_ms = start.elapsed().as_millis() as u64;
    let stdout = String::from_utf8_lossy(&output.stdout);

    if verbose {
        eprintln!("[agent] exit status: {}", output.status);
        eprintln!("[agent] raw stdout: {stdout}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.is_empty() {
            eprintln!("[agent] stderr: {stderr}");
        }
    }

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "claude CLI exited with {}: stderr={stderr}, stdout={stdout}",
            output.status
        );
    }

    parse_claude_output(&stdout, elapsed_ms, verbose)
}

pub fn parse_claude_output(raw_json: &str, elapsed_ms: u64, verbose: bool) -> Result<RunMetrics> {
    let v: serde_json::Value =
        serde_json::from_str(raw_json).context("failed to parse claude JSON output")?;

    if verbose {
        eprintln!(
            "[agent] parsed JSON keys: {:?}",
            v.as_object().map(|o| o.keys().collect::<Vec<_>>())
        );
    }

    let answer = v
        .get("result")
        .and_then(|r| r.as_str())
        .unwrap_or("")
        .to_string();

    let cost_usd = v.get("total_cost_usd").and_then(|c| c.as_f64());
    let num_turns = v.get("num_turns").and_then(|n| n.as_u64());

    let usage = v.get("usage");
    let input_tokens = usage
        .and_then(|u| u.get("input_tokens"))
        .and_then(|n| n.as_u64());
    let output_tokens = usage
        .and_then(|u| u.get("output_tokens"))
        .and_then(|n| n.as_u64());

    let tool_calls = None;

    if verbose {
        eprintln!(
            "[agent] extracted: cost={cost_usd:?} turns={num_turns:?} in_tok={input_tokens:?} out_tok={output_tokens:?}"
        );
    }

    Ok(RunMetrics {
        answer,
        elapsed_ms,
        input_tokens,
        output_tokens,
        tool_calls,
        cost_usd,
        num_turns,
    })
}
