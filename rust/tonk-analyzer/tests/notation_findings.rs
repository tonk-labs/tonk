//! Demo (notation tour): the two behaviours the tour found and worked
//! around, run as written in `rust/tonk-analyzer/tests/notation-tour/findings/`, so the
//! answers in `demo/notation-tour/findings.md` can be produced again.
//! They assert nothing; `cargo test -p tonk-analyzer --test
//! notation_findings -- --ignored --nocapture` prints what each gives.

#![cfg(not(target_arch = "wasm32"))]

use dialog_peer::helpers::{test_repo, test_session_with_peer};
use tonk_analyzer::notation::tour::{Cell, evaluate_on_branch};

async fn run(cells: &[&str]) {
    let (operator, profile) = test_session_with_peer().await;
    let repo = test_repo(&operator, &profile).await;
    let branch = repo
        .branch("main")
        .open()
        .perform(&operator)
        .await
        .expect("a branch");
    for (at, source) in cells.iter().enumerate() {
        let cell = Cell {
            source: (*source).to_owned(),
            context: cells[..at].iter().map(|cell| (*cell).to_owned()).collect(),
        };
        let answer = evaluate_on_branch(&branch, &cell, &operator).await;
        println!("=== cell {}\n{source}", at + 1);
        for form in &answer.forms {
            println!("--- {} | {}", form.kind, form.summary);
            for printed in &form.printed {
                println!("  {printed}");
            }
            if let Some(refused) = &form.refused {
                println!("  refused: {}", refused.message);
            }
        }
        if let Some(error) = &answer.error {
            println!("error: {error}");
        }
    }
}

#[dialog_common::test]
#[ignore = "prints a finding; asserts nothing"]
async fn a_formula_in_a_select_head() {
    run(&[include_str!(
        "notation-tour/findings/1-head-formula.notation"
    )])
    .await;
}

#[dialog_common::test]
#[ignore = "prints a finding; asserts nothing"]
async fn a_retract_through_a_concept() {
    run(&[
        include_str!("notation-tour/findings/2-retract-through-concept.notation"),
        include_str!("notation-tour/findings/2b-issue-and-read.notation"),
    ])
    .await;
}
