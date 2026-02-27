use anyhow::{Context, Result};
use std::path::Path;

use crate::types::ProbeResult;

pub fn write_results(results: &[ProbeResult], output_dir: &Path) -> Result<String> {
    let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    write_results_to(results, output_dir, &timestamp)
}

/// Write results to a file with a specific timestamp suffix.
///
/// Using a fixed timestamp means repeated calls (for partial results)
/// overwrite the same file rather than creating many files.
pub fn write_results_to(
    results: &[ProbeResult],
    output_dir: &Path,
    timestamp: &str,
) -> Result<String> {
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create output dir: {}", output_dir.display()))?;

    let filename = format!("results-{timestamp}.json");
    let filepath = output_dir.join(&filename);

    let json = serde_json::to_string_pretty(results).context("failed to serialize results")?;
    std::fs::write(&filepath, &json)
        .with_context(|| format!("failed to write {}", filepath.display()))?;

    Ok(filepath.display().to_string())
}

pub fn print_summary(results: &[ProbeResult]) {
    println!();
    println!(
        "{:<30} {:>6} {:>7} {:>8} {:>8} {:>9}",
        "PROBE", "SCORE", "TURNS", "IN TOK", "OUT TOK", "TIME ms"
    );
    println!("{}", "-".repeat(75));

    for r in results {
        let m = &r.run.metrics;
        let turns = m.num_turns.map_or("-".into(), |n| n.to_string());
        let in_tok = m.input_tokens.map_or("-".into(), |n| n.to_string());
        let out_tok = m.output_tokens.map_or("-".into(), |n| n.to_string());
        println!(
            "{:<30} {:>6} {:>7} {:>8} {:>8} {:>9}",
            truncate(&r.probe_id, 30),
            format!("{}/10", r.run.score.score),
            turns,
            in_tok,
            out_tok,
            m.elapsed_ms,
        );
    }

    println!("{}", "-".repeat(75));

    if !results.is_empty() {
        let n = results.len() as f64;
        let avg_score: f64 = results
            .iter()
            .map(|r| r.run.score.score as f64)
            .sum::<f64>()
            / n;
        let avg_turns: f64 = results
            .iter()
            .filter_map(|r| r.run.metrics.num_turns)
            .sum::<u64>() as f64
            / n;
        let total_in: u64 = results
            .iter()
            .filter_map(|r| r.run.metrics.input_tokens)
            .sum();
        let total_out: u64 = results
            .iter()
            .filter_map(|r| r.run.metrics.output_tokens)
            .sum();
        println!(
            "{:<30} {:>6} {:>7} {:>8} {:>8}",
            "TOTAL / AVG",
            format!("{avg_score:.1}/10"),
            format!("{avg_turns:.1}"),
            total_in,
            total_out,
        );
    }

    println!();
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}
