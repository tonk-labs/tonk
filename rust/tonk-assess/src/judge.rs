use anyhow::{Context, Result};
use tokio::process::Command;

use crate::types::{JudgeConfig, Score, ScoredKeyword};

// ── Public entry point ──────────────────────────────────────────────

pub async fn judge_answer(
    prompt: &str,
    answer: &str,
    judge: &JudgeConfig,
    default_model: &str,
    verbose: bool,
) -> Result<Score> {
    match judge {
        JudgeConfig::Llm {
            ground_truth,
            key_facts,
            system_prompt,
            model,
        } => {
            let model = model.as_deref().unwrap_or(default_model);
            let system = build_system_prompt(system_prompt.as_deref());
            llm_judge(
                prompt,
                ground_truth,
                key_facts,
                answer,
                model,
                &system,
                verbose,
            )
            .await
        }
        JudgeConfig::Keyword {
            keywords,
            max_score,
        } => Ok(keyword_judge(answer, keywords, *max_score, verbose)),
    }
}

// ── Keyword judge ───────────────────────────────────────────────────

fn keyword_judge(
    answer: &str,
    keywords: &[ScoredKeyword],
    max_score: Option<u32>,
    verbose: bool,
) -> Score {
    let answer_lower = answer.to_lowercase();
    let mut earned: u32 = 0;
    let mut matched = Vec::new();
    let mut missed = Vec::new();

    for kw in keywords {
        if answer_lower.contains(&kw.term.to_lowercase()) {
            earned += kw.score;
            matched.push(&kw.term);
        } else {
            missed.push(&kw.term);
        }
    }

    let total = max_score.unwrap_or_else(|| keywords.iter().map(|k| k.score).sum());

    let ratio = if total == 0 {
        0.0
    } else {
        earned as f64 / total as f64
    };

    let score = (ratio * 10.0).round() as u8;

    let rationale = format!(
        "{earned}/{total} points ({:.0}%). Matched: [{}]. Missed: [{}].",
        ratio * 100.0,
        matched
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(", "),
        missed
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(", "),
    );

    if verbose {
        eprintln!("[judge:keyword] {rationale}");
    }

    Score { score, rationale }
}

// ── LLM judge ───────────────────────────────────────────────────────

const JUDGE_SYSTEM_TEMPLATE: &str = include_str!("../prompt/judge/system.md");
const DEFAULT_RUBRIC: &str = include_str!("../prompt/judge/rubric.md");

fn build_system_prompt(rubric: Option<&str>) -> String {
    JUDGE_SYSTEM_TEMPLATE.replace("{rubric}", rubric.unwrap_or(DEFAULT_RUBRIC))
}

const SCORE_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "score": { "type": "integer", "minimum": 0, "maximum": 10 },
    "rationale": { "type": "string" }
  },
  "required": ["score", "rationale"],
  "additionalProperties": false
}"#;

async fn llm_judge(
    prompt: &str,
    ground_truth: &str,
    key_facts: &[String],
    answer: &str,
    model: &str,
    system_prompt: &str,
    verbose: bool,
) -> Result<Score> {
    let facts_section = if key_facts.is_empty() {
        String::new()
    } else {
        let list = key_facts
            .iter()
            .map(|f| format!("- {f}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("\n\nKey facts to check:\n{list}")
    };

    let user_prompt = format!(
        "Query: {prompt}\n\n\
         Ground truth: {ground_truth}{facts_section}\n\n\
         Answer to evaluate:\n{answer}"
    );

    let mut cmd = Command::new("claude");
    cmd.arg("-p")
        .arg("--output-format")
        .arg("json")
        .arg("--no-session-persistence")
        .arg("--max-turns")
        .arg("3")
        .arg("--model")
        .arg(model)
        .arg("--append-system-prompt")
        .arg(system_prompt)
        .arg("--json-schema")
        .arg(SCORE_SCHEMA)
        .arg("--")
        .arg(&user_prompt);

    cmd.env("CLAUDECODE", "");

    if verbose {
        eprintln!("[judge:llm] spawning claude CLI");
    }

    let output = cmd
        .output()
        .await
        .context("failed to spawn claude CLI for judging")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("judge claude CLI failed: {stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    if verbose {
        eprintln!("[judge:llm] raw output: {stdout}");
    }

    let envelope: serde_json::Value =
        serde_json::from_str(&stdout).context("failed to parse judge JSON output")?;

    parse_score(&envelope, verbose)
}

// ── Score parsing from claude JSON envelope ─────────────────────────

pub fn parse_score(envelope: &serde_json::Value, verbose: bool) -> Result<Score> {
    if let Some(structured) = envelope.get("structured_output") {
        if verbose {
            eprintln!("[judge:llm] structured_output: {structured}");
        }
        if let Ok(score) = serde_json::from_value::<Score>(structured.clone()) {
            return Ok(score);
        }
    }

    let result_str = envelope
        .get("result")
        .and_then(|r| r.as_str())
        .unwrap_or("");

    if verbose {
        eprintln!("[judge:llm] result field: {result_str}");
    }

    if let Ok(score) = serde_json::from_str::<Score>(result_str) {
        return Ok(score);
    }

    if let Some(json_str) = extract_json_from_markdown(result_str)
        && let Ok(score) = serde_json::from_str::<Score>(&json_str)
    {
        return Ok(score);
    }

    if let Some(score) = extract_score_from_text(result_str) {
        return Ok(score);
    }

    anyhow::bail!("failed to parse judge score from output. result field was: {result_str}")
}

fn extract_json_from_markdown(s: &str) -> Option<String> {
    let start = s.find("```")?;
    let after_ticks = &s[start + 3..];
    let content_start = after_ticks.find('\n')? + 1;
    let content = &after_ticks[content_start..];
    let end = content.find("```")?;
    Some(content[..end].trim().to_string())
}

fn extract_score_from_text(s: &str) -> Option<Score> {
    let mut depth = 0i32;
    let mut start = None;
    for (i, ch) in s.char_indices() {
        match ch {
            '{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    if let Some(s_idx) = start {
                        let candidate = &s[s_idx..=i];
                        if let Ok(score) = serde_json::from_str::<Score>(candidate) {
                            return Some(score);
                        }
                    }
                    start = None;
                }
            }
            _ => {}
        }
    }
    None
}
