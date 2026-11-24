//! Demo (notation tour): the two behaviours the tour found, run as written
//! in `rust/tonk-analyzer/tests/notation-tour/findings/`. Both are fixed at
//! the pinned dialog revision, and each test holds the fixed answer;
//! `--nocapture` prints what each cell gives, as `demo/notation-tour/findings.md`
//! quotes it.

#![cfg(not(target_arch = "wasm32"))]

use dialog_peer::helpers::{test_repo, test_session_with_peer};
use tonk_analyzer::notation::tour::{Cell, evaluate_on_branch};

async fn run(cells: &[&str]) -> Vec<Vec<String>> {
    let (operator, profile) = test_session_with_peer().await;
    let repo = test_repo(&operator, &profile).await;
    let branch = repo
        .branch("main")
        .open()
        .perform(&operator)
        .await
        .expect("a branch");
    let mut answers = Vec::new();
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
        for form in &answer.forms {
            if form.kind == "select" {
                answers.push(form.printed.clone());
            }
        }
    }
    answers
}

/// Finding 1: each pair answers with its own total, not a cross product.
#[dialog_common::test]
async fn a_formula_in_a_select_head() {
    let answers = run(&[include_str!(
        "notation-tour/findings/1-head-formula.notation"
    )])
    .await;
    assert_eq!(
        answers,
        vec![vec![
            "(tour.morning todo:milk total: 3 with: todo:seeds)".to_string(),
            "(tour.morning todo:milk total: 4 with: todo:deck)".to_string(),
            "(tour.morning todo:mum total: 3 with: todo:seeds)".to_string(),
            "(tour.morning todo:mum total: 4 with: todo:deck)".to_string(),
        ]]
    );
}

/// Finding 2: a record retracted and asserted changed in one round keeps
/// the fields both name (dialog-db/dialog-db#863).
#[dialog_common::test]
async fn a_retract_through_a_concept() {
    let answers = run(&[
        include_str!("notation-tour/findings/2-retract-through-concept.notation"),
        include_str!("notation-tour/findings/2b-issue-and-read.notation"),
    ])
    .await;
    assert_eq!(
        answers,
        vec![
            vec![
                "(tour.todo todo:milk list: list:home state: :done title: \"Buy milk\")"
                    .to_string()
            ],
            vec!["(tour.todo todo:milk title: \"Buy milk\")".to_string()],
            vec!["(tour.todo todo:milk state: :done)".to_string()],
            vec!["(tour.todo todo:milk list: list:home)".to_string()],
        ]
    );
}
