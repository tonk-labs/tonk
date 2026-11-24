//! Demo (notation tour): every ```notation cell of the tour runs.
//!
//! `demo/notation-tour/tour.md` is the notebook a "Notation tour" space
//! opens on. Each cell is run here in order, on one branch, the way the
//! notebook runs it (the cells above as its context), and a cell either
//! runs clean or, when it says `; expect: refused`, is refused. Set
//! `TONK_TOUR_PRINT=1` to print every answer.

#![cfg(not(target_arch = "wasm32"))]

use dialog_peer::helpers::{test_repo, test_session_with_peer};
use tonk_analyzer::notation::tour::{Cell, evaluate_on_branch};

const TOUR: &str = include_str!("../../../demo/notation-tour/tour.md");

/// The ```notation fences of a Markdown document, in order.
fn cells(markdown: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    for line in markdown.lines() {
        match &mut current {
            None if line.trim_start().starts_with("```notation") => current = Some(Vec::new()),
            Some(lines) if line.trim_start().starts_with("```") => {
                out.push(lines.join("\n"));
                current = None;
            }
            Some(lines) => lines.push(line),
            None => {}
        }
    }
    out
}

#[dialog_common::test]
async fn every_cell_of_the_tour_runs() {
    let (operator, profile) = test_session_with_peer().await;
    let repo = test_repo(&operator, &profile).await;
    let branch = repo
        .branch("main")
        .open()
        .perform(&operator)
        .await
        .expect("a branch");
    let print = std::env::var("TONK_TOUR_PRINT").is_ok();
    let cells = cells(TOUR);
    assert!(cells.len() > 5, "the tour has cells");
    let mut failures = Vec::new();
    for (at, source) in cells.iter().enumerate() {
        let cell = Cell {
            source: source.clone(),
            context: cells[..at].to_vec(),
        };
        let started = std::time::Instant::now();
        let answer = evaluate_on_branch(&branch, &cell, &operator).await;
        let took = started.elapsed();
        let refused =
            answer.error.is_some() || answer.forms.iter().any(|form| form.refused.is_some());
        let expected = source.contains("; expect: refused");
        let json = serde_json::to_string_pretty(&answer).expect("json");
        if print {
            println!(
                "=== cell {} ({took:?})\n{source}\n--- answer\n{json}\n",
                at + 1
            );
        }
        if refused != expected {
            failures.push(format!(
                "cell {} {}:\n{source}\n{json}",
                at + 1,
                if expected {
                    "should be refused and ran"
                } else {
                    "was refused"
                }
            ));
        }
        if source.contains("; expect: warning")
            && !answer.forms.iter().any(|form| !form.warnings.is_empty())
        {
            failures.push(format!("cell {}: no warning\n{source}\n{json}", at + 1));
        }
        // A cell that asks something and runs clean answers something.
        for form in &answer.forms {
            if form.refused.is_none()
                && form.summary.starts_with("0 ")
                && !source.contains("; expect: empty")
            {
                failures.push(format!("cell {}: `{}` answered nothing", at + 1, form.form));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
