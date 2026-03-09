use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tokio::signal;
use tokio_util::sync::CancellationToken;

use tonk_assess::types::{Probe, ProbeResult, Score, ScoredRun};
use tonk_assess::{agent, carry, judge, probe, report};

#[derive(Parser, Debug)]
#[command(
    name = "tonk-assess",
    about = "Benchmark harness for evaluating AI agent probe accuracy",
    long_about = "Runs probe questions against an AI agent and scores the answers.\n\n\
                   Each probe YAML defines the question, allowed tools, system prompt,\n\
                   and judge criteria. Use --list to see available probes.\n\n\
                   Examples:\n  \
                     tonk-assess benchmark/probe --persona marcus --tag lookup\n  \
                     tonk-assess benchmark/probe --persona marcus --probe marcus-lookup-01\n  \
                     tonk-assess benchmark/probe --list\n  \
                     tonk-assess benchmark/probe --list --judge keyword"
)]
struct Cli {
    /// Path to probe directory (required)
    probe_dir: PathBuf,

    /// Filter by persona name (required to run probes)
    #[arg(long)]
    persona: Option<String>,

    /// Filter by tag (probes must have all specified values)
    #[arg(long, value_delimiter = ',')]
    tag: Option<Vec<String>>,

    /// Filter by probe ID or keyword in prompt text
    #[arg(long)]
    probe: Option<String>,

    /// Filter by judge type (comma-delimited: llm, keyword, or llm,keyword)
    #[arg(long, value_delimiter = ',')]
    judge: Option<Vec<String>>,

    /// Model to use for agent runs
    #[arg(long, default_value = "claude-sonnet-4-6")]
    model: String,

    /// Model to use for LLM judge (when no model specified in probe)
    #[arg(long, default_value = "claude-sonnet-4-6")]
    judge_model: String,

    /// Path to output results directory
    #[arg(long)]
    output_dir: Option<PathBuf>,

    /// Print verbose debug output to stderr
    #[arg(long, short = 'v')]
    verbose: bool,

    /// List available probes and exit (does not run evaluation)
    #[arg(long)]
    list: bool,
}

/// Reason a probe was excluded by filters.
fn skip_reason(
    probe: &Probe,
    persona: Option<&str>,
    tag: Option<&[String]>,
    probe_filter: Option<&str>,
    judge_types: Option<&[String]>,
) -> Option<String> {
    if let Some(p) = persona
        && probe.persona != p
    {
        return Some(format!("persona: {}", probe.persona));
    }
    if let Some(required) = tag {
        for t in required {
            if !probe.tag.iter().any(|pt| pt == t) {
                return Some(format!("missing tag: {t}"));
            }
        }
    }
    if let Some(q) = probe_filter
        && !probe.id.contains(q)
        && !probe.prompt.to_lowercase().contains(&q.to_lowercase())
    {
        return Some("probe mismatch".to_string());
    }
    if let Some(types) = judge_types {
        let jt = probe.judge.judge_type();
        if !types.iter().any(|t| t == jt) {
            return Some(format!("judge: {jt}"));
        }
    }
    None
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let verbose = cli.verbose;

    let probe_dir = &cli.probe_dir;
    let output_dir = cli
        .output_dir
        .clone()
        .unwrap_or_else(|| probe_dir.parent().unwrap_or(probe_dir).join("results"));

    if verbose {
        eprintln!("[main] probe dir: {}", probe_dir.display());
        eprintln!("[main] output dir: {}", output_dir.display());
    }

    let all_probes = probe::load_probes(probe_dir)?;

    if cli.list {
        return list_probes(
            &all_probes,
            cli.persona.as_deref(),
            cli.tag.as_deref(),
            cli.probe.as_deref(),
            cli.judge.as_deref(),
            verbose,
        );
    }

    if cli.persona.is_none() && cli.tag.is_none() && cli.probe.is_none() {
        println!("tonk-assess: benchmark harness for evaluating AI agent probe accuracy\n");
        println!("To run an evaluation, specify at least one filter:\n");
        println!(
            "  tonk-assess benchmark/probe --persona marcus                    # all probes for marcus"
        );
        println!(
            "  tonk-assess benchmark/probe --persona marcus --tag lookup       # only lookup-tagged probes"
        );
        println!(
            "  tonk-assess benchmark/probe --probe marcus-lookup-01             # single probe by ID\n"
        );
        println!("Other commands:\n");
        println!(
            "  tonk-assess benchmark/probe --list                               # list available probes"
        );
        println!("  tonk-assess --help                               # full usage info");
        return Ok(());
    }

    let (matched, skipped): (Vec<_>, Vec<_>) = all_probes.into_iter().partition(|p| {
        skip_reason(
            p,
            cli.persona.as_deref(),
            cli.tag.as_deref(),
            cli.probe.as_deref(),
            cli.judge.as_deref(),
        )
        .is_none()
    });

    if !skipped.is_empty() {
        println!("Skipping {} probe(s):", skipped.len());
        for p in &skipped {
            let reason = skip_reason(
                p,
                cli.persona.as_deref(),
                cli.tag.as_deref(),
                cli.probe.as_deref(),
                cli.judge.as_deref(),
            )
            .unwrap_or_default();
            println!("  {} ({})", p.id, reason);
        }
        println!();
    }

    if matched.is_empty() {
        println!("No probes matched the given filters.");
        return Ok(());
    }

    println!("Found {} probe(s) to evaluate", matched.len());

    // ── Carry space provisioning ────────────────────────────────────
    // If any matched probes have carry-data, provision spaces before running.
    // Provisioning is resilient: failures for one persona don't block others.
    let has_carry_probes = matched.iter().any(|p| p.carry_data.is_some());
    let mut failed_personas: std::collections::HashSet<String> = std::collections::HashSet::new();

    if has_carry_probes {
        carry::ensure_available(verbose).await?;
        let (provisioned, failures) = carry::provision_all(&matched, probe_dir, verbose).await;
        if !failures.is_empty() {
            println!(
                "WARNING: Provisioning failed for {} persona(s):",
                failures.len()
            );
            for (persona, err) in &failures {
                println!("  {persona}: {err:#}");
                failed_personas.insert(persona.clone());
            }
            println!("  Carry probes for these personas will be skipped.\n");
        }
        if !provisioned.is_empty() {
            println!(
                "Provisioned {} Carry space(s): {}\n",
                provisioned.len(),
                provisioned.iter().cloned().collect::<Vec<_>>().join(", ")
            );
        }
    }

    println!("Press Ctrl-C to stop after the current probe.\n");

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    tokio::spawn(async move {
        let _ = signal::ctrl_c().await;
        eprintln!("\nInterrupted — finishing up...");
        cancel_clone.cancel();
    });

    // Track which persona's space is currently active
    let mut active_persona: Option<String> = None;

    let mut results: Vec<ProbeResult> = Vec::new();

    // Determine output path early so we can write partial results
    let results_timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();

    for probe in &matched {
        if cancel.is_cancelled() {
            println!("\nStopped early due to Ctrl-C.");
            break;
        }

        // Skip carry probes whose persona failed provisioning
        if probe.carry_data.is_some() && failed_personas.contains(&probe.persona) {
            println!(
                "--- {} --- SKIPPED (provisioning failed for '{}')\n",
                probe.name.as_deref().unwrap_or(&probe.id),
                probe.persona
            );
            continue;
        }

        // Switch active Carry space if this probe uses carry-data and
        // the persona differs from the currently active one.
        if probe.carry_data.is_some() {
            let need_switch = active_persona.as_ref() != Some(&probe.persona);
            if need_switch {
                match carry::set_active_space(&probe.persona, verbose).await {
                    Ok(()) => {
                        active_persona = Some(probe.persona.clone());
                    }
                    Err(e) => {
                        eprintln!(
                            "--- {} --- SKIPPED (failed to switch Carry space: {e:#})\n",
                            probe.name.as_deref().unwrap_or(&probe.id),
                        );
                        continue;
                    }
                }
            }
        }

        let label = probe.name.as_deref().unwrap_or(&probe.id);
        println!("--- {} ---", label);
        println!("Q: {}", probe.prompt);

        // Resolve corpus directory:
        // 1. If probe has corpus, resolve it relative to probe_dir
        // 2. Fall back to <probe_dir>/../personas/<persona>/artifacts
        let corpus_dir = if let Some(ref rel) = probe.corpus {
            probe_dir.join(rel)
        } else {
            probe_dir
                .parent()
                .unwrap_or(probe_dir)
                .join("personas")
                .join(&probe.persona)
                .join("artifacts")
        };

        println!("  Running agent...");
        match run_cancellable(
            &cancel,
            agent::run_probe(probe, &cli.model, probe_dir, &corpus_dir, verbose),
        )
        .await
        {
            Some(Ok(metrics)) => {
                println!("  Answer: {}", truncate_answer(&metrics.answer, 120));
                println!("  Judging...");
                match judge::judge_answer(
                    &probe.prompt,
                    &metrics.answer,
                    &probe.judge,
                    &cli.judge_model,
                    verbose,
                )
                .await
                {
                    Ok(score) => {
                        let in_tok = metrics.input_tokens.map_or("-".into(), |n| n.to_string());
                        let out_tok = metrics.output_tokens.map_or("-".into(), |n| n.to_string());
                        println!("  Score: {}/10 — {}", score.score, score.rationale);
                        println!("  Tokens: {in_tok} in / {out_tok} out");
                        results.push(ProbeResult {
                            probe_id: probe.id.clone(),
                            persona: probe.persona.clone(),
                            tag: probe.tag.clone(),
                            prompt: probe.prompt.clone(),
                            run: ScoredRun { metrics, score },
                        });
                    }
                    Err(e) => {
                        eprintln!("  Judge error (skipping): {e:#}");
                        // Record with score 0 so the probe appears in results
                        let score = Score {
                            score: 0,
                            rationale: format!("JUDGE ERROR: {e:#}"),
                        };
                        results.push(ProbeResult {
                            probe_id: probe.id.clone(),
                            persona: probe.persona.clone(),
                            tag: probe.tag.clone(),
                            prompt: probe.prompt.clone(),
                            run: ScoredRun { metrics, score },
                        });
                    }
                }
            }
            Some(Err(e)) => {
                eprintln!("  Error: {e:#}");
            }
            None => {
                println!("\nStopped early due to Ctrl-C.");
                break;
            }
        }

        println!();

        // Write partial results after each probe so progress is never lost
        if !results.is_empty()
            && let Err(e) = report::write_results_to(&results, &output_dir, &results_timestamp)
            && verbose
        {
            eprintln!("[main] Failed to write partial results: {e:#}");
        }
    }

    if !results.is_empty() {
        report::print_summary(&results);
        match report::write_results_to(&results, &output_dir, &results_timestamp) {
            Ok(path) => println!("Results written to {path}"),
            Err(e) => eprintln!("WARNING: Failed to write results: {e:#}"),
        }
    }

    Ok(())
}

async fn run_cancellable<F, T>(cancel: &CancellationToken, fut: F) -> Option<T>
where
    F: std::future::Future<Output = T>,
{
    tokio::select! {
        biased;
        _ = cancel.cancelled() => None,
        result = fut => Some(result),
    }
}

fn list_probes(
    all_probes: &[Probe],
    persona: Option<&str>,
    tag: Option<&[String]>,
    probe_filter: Option<&str>,
    judge_types: Option<&[String]>,
    verbose: bool,
) -> Result<()> {
    let matched: Vec<_> = all_probes
        .iter()
        .filter(|p| skip_reason(p, persona, tag, probe_filter, judge_types).is_none())
        .collect();

    if matched.is_empty() {
        println!("No probes found.");
        return Ok(());
    }

    for p in &matched {
        let label = p.name.as_deref().unwrap_or(&p.id);
        println!("  {}: {}", label, p.prompt);
        if verbose {
            println!("    id:    {}", p.id);
            if !p.tag.is_empty() {
                println!("    tag:   {}", p.tag.join(", "));
            }
            println!("    judge: {}", p.judge.judge_type());
            if let Some(ref corpus) = p.corpus {
                println!("    corpus: {corpus}");
            }
            if !p.allowed_tools.is_empty() {
                println!("    tools: {}", p.allowed_tools.join(", "));
            }
            if let Some(ref sp) = p.system_prompt {
                println!("    system-prompt: {sp}");
            }
            if let Some(turns) = p.max_turns {
                println!("    max-turns: {turns}");
            }
            if let Some(ref cd) = p.carry_data {
                println!("    carry-data: {cd}");
            }
            if let Some(ref cm) = p.carry_model {
                println!("    carry-model: {cm}");
            }
            println!();
        }
    }
    println!("\n{} probe(s)", matched.len());

    Ok(())
}

fn truncate_answer(s: &str, max: usize) -> String {
    let oneline = s.replace('\n', " ");
    if oneline.len() <= max {
        oneline
    } else {
        format!("{}…", &oneline[..max - 1])
    }
}
